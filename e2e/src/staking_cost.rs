//! Worst-case execution cost of the two hot staking system-call paths.
//!
//! Self-contained: nothing outside this file is referenced by it, and nothing
//! outside references it except the `mod` declaration in `lib.rs`.
//!
//! Everything here runs through real rWasm in the e2e harness, so the printed
//! numbers are measured gas, not estimates. The harness charges the EVM
//! transaction intrinsic (21_000 + calldata) on top of the frame; a real system
//! call does not (`revm-rwasm` `crates/handler/src/handler.rs:128` builds
//! `InitialAndFloorGas::new(0, 0)`), so each measurement reports the frame-only
//! figure alongside the raw transaction figure.

use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolEvent};
use fluentbase_sdk::{
    address, hex, Address, Bytes, B256, GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_testing::EvmTestingContext;
use revm::Database;

const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
const OWNER: Address = Address::repeat_byte(0x11);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

/// Far above any real budget: a measurement must never be clipped by the limit.
const HUGE_GAS: u64 = 2_000_000_000;

/// Committee-size cap (`MAX_ACTIVE_VALIDATORS_LENGTH`).
const COMMITTEE: usize = 51;
/// `epochBlockInterval`, equal to `COMMITTEE` so that a floor of one due block
/// makes every member judgeable at an even stake split.
const INTERVAL: u64 = COMMITTEE as u64;
/// Must be a multiple of `INTERVAL`, which the config validator enforces.
const ACTIVATION: u64 = 51 * 20;

/// Per-call budget of a system call: `revm-rwasm` `crates/handler/src/system_call.rs:64`
/// builds every one of them with `gas_limit(30_000_000)`.
const SYSTEM_CALL_BUDGET: f64 = 30_000_000.0;
/// `STIPEND_FUEL_CAP` in gas: `12_000_000 * FUEL_DENOM_RATE` fuel, and fuel
/// converts back at the same rate (`crates/revm/src/executor.rs:420`).
const STIPEND_CAP_GAS: u64 = 12_000_000;
/// Mirrors the contract's `consts.rs::MAX_SETTLE_CATCHUP`. Epochs one settlement
/// call may fund, and therefore the number of token pulls the two "maximum
/// catch-up" measurements below must be able to count.
const MAX_SETTLE_CATCHUP: u64 = 4;

// ---------------------------------------------------------------------------
// Cost guards.
//
// Every threshold below is a LITERAL with headroom over what this harness
// measured on real rWasm, recorded in the doc comment beside it; every guard
// in this file has been seen to go red, by moving its threshold one unit past
// the measurement.
// None is derived from the measurement at run time: a bound computed from the
// number it bounds cannot fail, which is the `MAX + 1` shape this suite has
// already had to remove twice.
//
// Gas here is deterministic — the same contract on the same input charges the
// same gas on every run — so the headroom is not for noise, of which there is
// none. It is for benign change: a repacked storage slot, one more field on an
// event, a reordered read. ~15% on the cost ceilings is wide enough that such a
// change does not cost the next person an afternoon, and far too narrow to hide
// a doubling of the judge sweep, the roster scan, or the stipend leg.
//
// The roster floors are the operationally meaningful number: the point at which
// the unpruned roster makes the call exceed its 30M budget and the block stops
// executing. They sit ~12% below today, which is deliberately tighter than the
// conjunction of the matching intercept and slope ceilings — an intercept at
// its ceiling paired with a slope just under its own would still clear both and
// fail the floor. That keeps the floor load-bearing instead of decorative.
//
// READ THE THREE FLOORS AS A MINIMUM, NEVER INDIVIDUALLY. The roster is walked
// by two separate system calls, each with its own 30M, and the chain stops at
// whichever runs out first — today that is `commitEpochCommittee`, ~2.6x
// tighter than the epoch close. The commit binds because it pays the view's
// per-entry slope on top of a ~2.5M intercept, while the close pays a slope a
// third that size. Quoting the close's headroom as "the" limit overstates it by
// that factor.
//
// That gap WIDENED when the stipend split moved into the close (2026-08-17): the
// close's intercept fell by 47% and its floor rose from 4_871 to 5_643 while the
// commit did not move at all — 2_574_893 both before and after, to the digit, so
// the "~2.5M" above is the same number it has always described. The close used to
// have the larger fixed cost of the two by nearly 3x; it is now only ~1.6x the
// commit's, and the two are on converging paths. The minimum rule matters more
// than it did, not less.
//
// Crossing it is not recoverable in-band: the commit is a pre-execution system
// call, so the block fails before any transaction in it runs, and the
// transaction that would prune the roster can never be mined.
// ---------------------------------------------------------------------------

/// Measured 2026-08-17: 4_041_657. Was 7_594_252 on 2026-08-06.
///
/// It FELL by 3_552_595, and the ceiling is re-pinned down to match rather than
/// left where it was. A ceiling three times the measurement reads green through
/// any regression short of a tripling, so leaving 8_730_000 in place would have
/// silently spent the whole improvement this change bought.
///
/// The fall is the stipend split moving out of the payment leg and into the
/// close: the leg shed 6_603_076 (see `STIPEND_LEG_MAX`) while the close took on
/// 3_264_563 of accrual (see `PATH_A_ACCRUAL_MAX`). The close does the work for
/// ONE epoch where the leg did it for `MAX_SETTLE_CATCHUP`, which is where the
/// net saving comes from — not from the work getting cheaper.
const PATH_A_INTERCEPT_MAX: f64 = 4_640_000.0;
/// Measured 2026-08-17: 4_600 gas per roster entry, unchanged from 2026-08-06.
///
/// Left at its existing value deliberately: the roster walk is `release_expired`
/// and the exclusion sweep, neither of which the stipend split touches, and the
/// measurement confirms it to the digit. 5_300 is already ~15% over it.
const PATH_A_SLOPE_MAX: f64 = 5_300.0;
/// Measured 2026-08-17: roster 5_643. Was 4_871 on 2026-08-06.
///
/// A floor that RISES is the one direction that cannot be left alone: 4_280 was
/// 12% under the old measurement and is 24% under this one, so it would no longer
/// catch the close regressing back to where it was.
const PATH_A_ROSTER_FLOOR: f64 = 4_960.0;

/// Measured 2026-08-17: 3_264_563 for a fresh epoch, 811_463 for one whose
/// per-validator snapshots already exist.
///
/// Pinned against the fresh-epoch figure, which is the steady state: every epoch
/// is new. Three quarters of it is `touch_snapshot_at_or_before` materializing 51
/// snapshots at ~48_100 each, and that is work the settlement leg used to do —
/// moved, not added.
///
/// This replaces a ~1.5M estimate that was decomposed from other measurements
/// rather than measured. The estimate was low by 2.2x.
const PATH_A_ACCRUAL_MAX: u64 = 3_750_000;

/// Measured 2026-08-06: 544_000.
const VIEW_INTERCEPT_MAX: f64 = 625_000.0;
/// Measured 2026-08-06: 12_800 gas per roster entry.
const VIEW_SLOPE_MAX: f64 = 14_700.0;
/// Measured 2026-08-06: roster 2_301.
const VIEW_ROSTER_FLOOR: f64 = 2_020.0;

/// Measured 2026-08-06: 2_574_893.
///
/// This rose by ~143k when the committee and its frozen weights merged into one
/// vector of pairs. The merge was made for safety — a misaligned pairing is now
/// unrepresentable rather than length-checked — and it costs slightly more to
/// write than two parallel pushes did. Recorded so the next reader does not
/// mistake a deliberate price for a regression.
const COMMIT_INTERCEPT_MAX: f64 = 2_960_000.0;
/// The same walk the view above pays for, so the same slope to the digit: both
/// rank the whole roster. Note that these are two independent ceilings against
/// one literal, so they catch a slope that RISES; a commit that stopped
/// re-deriving would send this slope toward zero and both guards would still
/// read green. Detecting that needs the two fitted slopes compared against each
/// other, which no assertion here does.
///
/// Measured 2026-08-06: 12_800 gas per roster entry.
const COMMIT_SLOPE_MAX: f64 = 14_700.0;
/// The binding limit of the three — see the minimum rule above.
///
/// Measured 2026-08-06: roster 2_143.
const COMMIT_ROSTER_FLOOR: f64 = 1_885.0;

/// Measured 2026-08-17: 48_852, i.e. 0.4% of the cap. Was 6_651_928 — 55.4% of
/// it — on 2026-08-06.
///
/// A fall of 6_603_076, or 99.27%. The leg no longer splits a pot or walks a
/// committee; it reads one scalar per epoch and transfers it, so its cost is
/// `MAX_SETTLE_CATCHUP` × ~12_200 and is independent of committee size.
///
/// **This guard's job changed with that, and the number is not a percentage band
/// over the measurement.** The old 8_000_000 was 20% over a figure that was
/// genuinely approaching the contract's 12M cap. At 48_852 the leg cannot
/// approach that cap by growing — it would have to change shape — so a
/// percentage band would only fire on benign per-epoch edits (one more event, one
/// more read), and 8_000_000 would let it grow 164x unremarked. Neither is a
/// guard.
///
/// 250_000 is set to catch the regression that actually matters: a per-member
/// walk returning to the payment path. One committee-sized walk at 51 members
/// costs on the order of 1M, so any such change trips this immediately, while
/// ~5.1x of today absorbs several extra storage operations per settled epoch.
///
/// The consequence of crossing the contract's own cap is still silent — the
/// self-call takes `OutOfFuel`, `settle_stipend_leg` swallows it as a status —
/// but it now costs only a deferral: the accrual ran in the outer frame and the
/// epochs stay owed.
const STIPEND_LEG_MAX: u64 = 250_000;

const _: () = assert!(
    STIPEND_LEG_MAX < STIPEND_CAP_GAS,
    "the stipend guard must trip before the contract's own cap does"
);

/// Collects threshold violations instead of panicking on the first one.
///
/// A regression usually moves several of these together, and stopping at the
/// first tells the next reader the least useful thing. Every guard is evaluated
/// and printed; the assert at the end reports all of them at once.
#[derive(Default)]
struct Guards {
    violations: Vec<String>,
}

impl Guards {
    fn at_most(&mut self, what: &str, measured: f64, ceiling: f64) {
        let ok = measured <= ceiling;
        println!(
            "  [{}] {what}: {measured:.0} <= {ceiling:.0}",
            if ok { "ok" } else { "TRIPPED" }
        );
        if !ok {
            self.violations.push(format!(
                "{what}: {measured:.0} exceeds the ceiling {ceiling:.0}"
            ));
        }
    }

    fn at_least(&mut self, what: &str, measured: f64, floor: f64) {
        let ok = measured >= floor;
        println!(
            "  [{}] {what}: {measured:.0} >= {floor:.0}",
            if ok { "ok" } else { "TRIPPED" }
        );
        if !ok {
            self.violations.push(format!(
                "{what}: {measured:.0} is below the floor {floor:.0}"
            ));
        }
    }

    fn finish(self) {
        assert!(
            self.violations.is_empty(),
            "staking cost guards tripped ({} of them):\n  - {}\n\
             \nThresholds are in e2e/src/staking_cost.rs with the figure each was \
             set against. Decide whether the cost regressed or the threshold has \
             simply aged, and say which in the commit.",
            self.violations.len(),
            self.violations.join("\n  - "),
        );
    }
}

/// Least-squares fit over every measured point, returned as `(intercept, slope)`.
///
/// Endpoints alone would let a middle point drift unseen, and the middle points
/// are the ones that would reveal a cost that is not actually linear in the
/// roster.
fn linear_fit(points: &[(usize, u64)]) -> (f64, f64) {
    let count = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| *x as f64).sum::<f64>() / count;
    let mean_y = points.iter().map(|(_, y)| *y as f64).sum::<f64>() / count;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for (x, y) in points {
        let dx = *x as f64 - mean_x;
        covariance += dx * (*y as f64 - mean_y);
        variance += dx * dx;
    }
    let slope = covariance / variance;
    (mean_y - slope * mean_x, slope)
}

/// Roster size at which the fitted line reaches the 30M system-call budget.
fn roster_at_budget(intercept: f64, slope: f64) -> f64 {
    (SYSTEM_CALL_BUDGET - intercept) / slope
}

sol! {
    struct EpochConsensusKeys {
        bytes blsPubkey;
        bytes32 peerPubkey;
        uint64 activationEpoch;
    }

    /// The close's accrual fact: what the epoch owes, decided at its close.
    event EpochBlendRewardsCommitted(uint64 indexed epoch, uint256 blendAmount);
    /// The payment paid nobody for this epoch, whether because the close owed
    /// nothing or because no close will ever run for it.
    event StipendSkipped(uint64 indexed epoch);
    /// The fuel-capped payment frame was discarded. Emitted from the outer frame,
    /// which is why it survives when the two events above do not.
    event StipendLegSkipped(uint64 indexed epoch);

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
            address blsVerifier,
            uint256 minUndelegateBlocks,
            address blendReserve
        ) external;

        function recordProduction(uint8 leaderIndex) external;
        function commitEpochCommittee() external;
        function settleEpochStipend(uint64 epoch) external;

        function getValidatorsWithKeysAt(uint64 epoch) external view
            returns (address[] validators, EpochConsensusKeys[] keys);
        function blocksInEpoch(uint64 epoch) external view returns (uint32);
        function pendingExclusions() external view returns (address[]);
        function getEpochRewards(uint64 epoch) external view returns (uint256);

        function setProductionLivenessDisabled(bool value) external;
        function setMinVerdictDueBlocks(uint32 value) external;
        function setBlendStipendPerEpoch(uint256 value) external;
        function setBlendReserve(address value) external;
    }
}

// ---------------------------------------------------------------------------
// Hand-written EVM mocks. There is no solc in this tree, and the vendored
// compiled mock returns one constant compressed key for every validator, which
// the contract's key-uniqueness check rejects from the second validator on.
// ---------------------------------------------------------------------------

/// Deploys runtime bytecode behind a minimal constructor prologue.
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

/// `compressG2Unchecked(bytes)` echoes the first 96 bytes of its argument, so
/// distinct uncompressed keys give distinct compressed keys. Every other
/// selector returns `true`.
fn deploy_bls_verifier(context: &mut EvmTestingContext) -> Address {
    deploy_runtime(
        context,
        &hex!(
            // selector == compressG2Unchecked(bytes) ?
            "60003560e01c63a5d2dd2214601957"
            // no: return true
            "600160005260206000f3"
            // yes:
            "5b"
            // head: offset 0x20, length 0x60
            "60206000526060602052"
            // body: calldata[0x44..0xa4], return 0xa0 bytes
            "6060604460403760a06000f3"
        ),
    )
}

/// ERC-20 stand-in that models one allowance and nothing else: `transferFrom`
/// succeeds for `OWNER` and reverts for anybody else, every other call returns
/// `1`. That is the whole surface the stipend needs now — it pulls the pot off
/// the configured address, so an address that has not approved the staking
/// contract is exactly an address whose `transferFrom` reverts. No balances are
/// tracked; this measures gas, not solvency.
///
/// It DOES count successful pulls, in storage slot zero, and that counter is the
/// only positive evidence in this file that a measured frame did the settlement
/// work it is being priced for. Read it with `committed_pulls`. Being storage, it
/// unwinds with a discarded frame, which is exactly the discrimination needed:
/// the stipend leg's self-call is the one frame here whose failure is swallowed.
fn deploy_token(context: &mut EvmTestingContext) -> Address {
    deploy_runtime(
        context,
        &hex!(
            // selector == transferFrom(address,address,uint256) ?
            "60003560e01c6323b872dd14601957"
            // no: return true
            "600160005260206000f3"
            // yes: is the `from` argument OWNER?
            "5b"
            "60043573"
            "1111111111111111111111111111111111111111"
            "14603b57"
            // no: revert, which is what an unapproved source looks like
            "60006000fd"
            // yes: sstore(0, sload(0) + 1), then return true
            "5b"
            "600054600101600055"
            "600160005260206000f3"
        ),
    )
}

/// Successful `transferFrom` calls the token has COMMITTED, ever.
///
/// Slot zero of the mock. A pull inside a frame that was later discarded is not
/// counted, because the store went with the frame.
fn committed_pulls(context: &mut EvmTestingContext, token: Address) -> u64 {
    context
        .db
        .storage(token, U256::ZERO)
        .expect("the mock token's pull counter is readable")
        .to()
}

/// An address that never approved the staking contract, used to stall the
/// settlement cursor so that a later close has to catch up over several epochs.
const UNAPPROVED: Address = Address::repeat_byte(0x99);

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

fn set_block(context: &EvmTestingContext, number: u64) {
    // `TestingContextImpl` is an `Rc<RefCell<..>>` handle, so the clone mutates
    // the same inner state the harness reads when it builds the block env.
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
    output: Vec<u8>,
    /// Gas actually spent during execution, before refunds. This is the figure a
    /// gas limit is checked against while the frame runs, so it is the one the
    /// 30M budget has to cover.
    tx_gas: u64,
    /// The same, minus the transaction intrinsic the harness charges and a
    /// system call does not: what the system call itself spends.
    frame_gas: u64,
    /// Post-refund figure, reported only so the gap is visible.
    tx_gas_after_refund: u64,
    /// `topic0` of every log the call committed, in order.
    ///
    /// A log written inside a discarded frame is not here, which is what lets an
    /// assertion tell a payment that happened from one that was rolled back.
    event_topics: Vec<B256>,
}

impl Measured {
    fn events(&self, signature_hash: B256) -> usize {
        self.event_topics
            .iter()
            .filter(|topic| **topic == signature_hash)
            .count()
    }
}

fn measure(context: &mut EvmTestingContext, caller: Address, input: Vec<u8>) -> Measured {
    let overhead = intrinsic(&input);
    let result = context.call_evm_tx(caller, GENESIS_STAKING, input.into(), Some(HUGE_GAS), None);
    assert!(result.is_success(), "call failed: {result:?}");
    let tx_gas = result.gas().total_gas_spent();
    Measured {
        output: result.output().cloned().unwrap_or_default().to_vec(),
        tx_gas,
        frame_gas: tx_gas.saturating_sub(overhead),
        tx_gas_after_refund: result.tx_gas_used(),
        event_topics: result
            .logs()
            .iter()
            .filter_map(|log| log.topics().first().copied())
            .collect(),
    }
}

fn call(context: &mut EvmTestingContext, caller: Address, input: Vec<u8>) -> Vec<u8> {
    measure(context, caller, input).output
}

fn validator_address(index: usize) -> Address {
    let mut bytes = [0u8; 20];
    bytes[0] = 0xa0;
    bytes[18] = (index >> 8) as u8;
    bytes[19] = index as u8;
    Address::from(bytes)
}

/// Strictly ascending in `index`, which is the order `commitEpochCommittee`
/// sorts the committee into before storing it.
fn peer_pubkey(index: usize) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[30] = ((index + 1) >> 8) as u8;
    bytes[31] = (index + 1) as u8;
    B256::from(bytes)
}

fn bls_pubkey(index: usize) -> Bytes {
    let mut bytes = vec![0x11u8; 256];
    bytes[0] = 0xc0;
    bytes[1] = (index >> 8) as u8;
    bytes[2] = index as u8;
    Bytes::from(bytes)
}

struct Fixture {
    context: EvmTestingContext,
    validators: Vec<Address>,
    token: Address,
}

/// Genesis with `roster` active, equally staked, key-carrying validators.
///
/// They are seeded through `initialize`, the only path that makes a validator
/// Active and selection-visible from epoch 0 with no warm-up delay, so the
/// committee can be committed immediately.
fn fixture(roster: usize, source_is_approved: bool) -> Fixture {
    let mut context = EvmTestingContext::default().with_full_genesis();
    set_block(&context, ACTIVATION - 1);

    let verifier = deploy_bls_verifier(&mut context);
    let token = deploy_token(&mut context);

    let validators: Vec<Address> = (0..roster).map(validator_address).collect();
    let calldata = IStaking::initializeCall {
        initialStakeOwner: OWNER,
        validators: validators.clone(),
        initialStakes: vec![TOKEN * U256::from(10); roster],
        blsPubkeysUncompressed: (0..roster).map(bls_pubkey).collect(),
        blsPopsUncompressed: vec![vec![0x22u8; 128].into(); roster],
        peerPubkeys: (0..roster).map(peer_pubkey).collect(),
        commissionRate: 0,
        stakingToken: token,
        activeValidatorsLength: COMMITTEE as u32,
        epochBlockInterval: INTERVAL as u32,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: ACTIVATION,
        blsVerifier: verifier,
        minUndelegateBlocks: U256::ZERO,
        blendReserve: if source_is_approved {
            OWNER
        } else {
            UNAPPROVED
        },
    }
    .abi_encode();
    call(&mut context, GENESIS_GOVERNANCE, calldata);

    call(
        &mut context,
        GENESIS_GOVERNANCE,
        IStaking::setBlendStipendPerEpochCall {
            value: TOKEN * U256::from(COMMITTEE),
        }
        .abi_encode(),
    );

    Fixture {
        context,
        validators,
        token,
    }
}

impl Fixture {
    /// Successful `transferFrom` calls the token has committed so far.
    fn committed_pulls(&mut self) -> u64 {
        committed_pulls(&mut self.context, self.token)
    }

    fn commit_committee(&mut self) -> Measured {
        measure(
            &mut self.context,
            SYSTEM_CALLER,
            IStaking::commitEpochCommitteeCall {}.abi_encode(),
        )
    }

    fn record(&mut self, block: u64) -> Measured {
        set_block(&self.context, block);
        measure(
            &mut self.context,
            SYSTEM_CALLER,
            IStaking::recordProductionCall { leaderIndex: 0 }.abi_encode(),
        )
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

    fn pending_exclusions(&mut self) -> Vec<Address> {
        let output = call(
            &mut self.context,
            OWNER,
            IStaking::pendingExclusionsCall {}.abi_encode(),
        );
        IStaking::pendingExclusionsCall::abi_decode_returns(&output).unwrap()
    }
}

/// First block of `epoch`.
fn first_block(epoch: u64) -> u64 {
    ACTIVATION + epoch * INTERVAL
}

// ---------------------------------------------------------------------------
// Path B: the committee-selection view the node reads at every epoch boundary.
// ---------------------------------------------------------------------------

#[test]
fn path_b_selection_view_cost_by_roster_size() {
    println!("\n=== PATH B: getValidatorsWithKeysAt(epoch), by selection-roster size ===");
    println!(
        "{:>8}  {:>16}  {:>16}",
        "roster", "view frame gas", "commit frame gas"
    );
    let mut points: Vec<(usize, u64)> = Vec::new();
    let mut commit_points: Vec<(usize, u64)> = Vec::new();
    for roster in [COMMITTEE, 100, 200, 400] {
        let mut fixture = fixture(roster, true);
        let measured = measure(
            &mut fixture.context,
            OWNER,
            IStaking::getValidatorsWithKeysAtCall { epoch: 0 }.abi_encode(),
        );
        let decoded = IStaking::getValidatorsWithKeysAtCall::abi_decode_returns(&measured.output)
            .expect("selection view decodes");
        assert_eq!(decoded.validators.len(), COMMITTEE);
        // The write half of the same path: it re-derives the identical selection
        // and freezes it.
        let commit = fixture.commit_committee();
        println!(
            "{roster:>8}  {:>16}  {:>16}",
            measured.frame_gas, commit.frame_gas
        );
        points.push((roster, measured.frame_gas));
        commit_points.push((roster, commit.frame_gas));
    }
    let mut guards = Guards::default();

    let (view_base, view_slope) = linear_fit(&points);
    let view_roster = roster_at_budget(view_base, view_slope);
    println!("\ngetValidatorsWithKeysAt fit: {view_base:.0} + {view_slope:.0} * roster");
    println!("view exhausts the 30M budget at roster ~= {view_roster:.0}");
    guards.at_most("view intercept", view_base, VIEW_INTERCEPT_MAX);
    guards.at_most("view gas per roster entry", view_slope, VIEW_SLOPE_MAX);
    guards.at_least("view roster headroom", view_roster, VIEW_ROSTER_FLOOR);

    let (commit_base, commit_slope) = linear_fit(&commit_points);
    let commit_roster = roster_at_budget(commit_base, commit_slope);
    println!("\ncommitEpochCommittee fit: {commit_base:.0} + {commit_slope:.0} * roster");
    println!("commit exhausts the 30M budget at roster ~= {commit_roster:.0}");
    guards.at_most("commit intercept", commit_base, COMMIT_INTERCEPT_MAX);
    guards.at_most(
        "commit gas per roster entry",
        commit_slope,
        COMMIT_SLOPE_MAX,
    );
    guards.at_least("commit roster headroom", commit_roster, COMMIT_ROSTER_FLOOR);

    guards.finish();
}

// ---------------------------------------------------------------------------
// Path A: recordProduction at an epoch boundary.
// ---------------------------------------------------------------------------

/// Full committee, every judgeable member failing, maximum stamps, maximum
/// releases, maximum settlement catch-up, all in one close.
///
/// The schedule:
///   epochs 0..=1  liveness tier off and the stipend drawn from an address that
///                 never approved the contract, so nothing is judged and the
///                 settlement cursor stays at epoch 0.
///   epoch 2       tier armed mid-epoch.
///   close(2)      50 first-time failures trip the correlation guard: verdicts
///                 are stamped, no exclusion is applied.
///   close(3)      the same 50 fail again as repeats, the guard stands down,
///                 `stamp` applies two exclusions.
///   close(4)      WORST. Judges 51 again, releases the two exclusions that
///                 expire, applies two more, and — the approved source having
///                 been configured mid-epoch — settles epochs 0..=3, which is
///                 `MAX_SETTLE_CATCHUP`.
#[test]
fn path_a_record_production_epoch_close_worst_case() {
    println!("\n=== PATH A: recordProduction at an epoch boundary ===");
    let mut points: Vec<(usize, u64)> = Vec::new();
    // Strictly greater than the cap, or `apply_production_exclusion` refuses
    // every stamp: excluding the marginal candidate would shrink the committee
    // instead of rotating it.
    for roster in [COMMITTEE + 4, 100, 200, 400] {
        points.push((roster, worst_case_close(roster)));
    }
    let (base, per_entry) = linear_fit(&points);
    let roster_at_30m = roster_at_budget(base, per_entry);
    println!("\nfit: worst close frame_gas ~= {base:.0} + {per_entry:.0} * roster");
    println!("30M budget exhausted at roster ~= {roster_at_30m:.0}");
    // The 18M framing applies to the part of the close that is NOT the
    // fuel-capped stipend leg. That part is what `close(epoch 3)` above
    // measures: judge plus two stamps, with the stipend leg failing fast.
    println!(
        "note: the 18M residual applies to the non-stipend part, printed above \
         as close(epoch 3); the WORST figure includes the stipend leg's actual \
         consumption, not its 12M reservation"
    );

    let mut guards = Guards::default();
    guards.at_most("path A intercept", base, PATH_A_INTERCEPT_MAX);
    guards.at_most("path A gas per roster entry", per_entry, PATH_A_SLOPE_MAX);
    guards.at_least("path A roster headroom", roster_at_30m, PATH_A_ROSTER_FLOOR);
    guards.finish();
}

fn worst_case_close(roster: usize) -> u64 {
    let mut fixture = fixture(roster, false);
    println!("\n-- roster {roster}, committee {COMMITTEE}, interval {INTERVAL}");

    // Committees 0..=2 can be committed at once; the lookahead is two epochs, so
    // 3..=5 have to be issued from inside epochs 1..=3.
    for _ in 0..3 {
        fixture.commit_committee();
    }

    let mut baseline = 0u64;
    for epoch in 0..5u64 {
        let start = first_block(epoch);
        for offset in 0..INTERVAL {
            let block = start + offset;
            let measured = fixture.record(block);
            if epoch == 0 && offset == 1 {
                baseline = measured.frame_gas;
            }
            if offset == 0 && epoch > 0 {
                println!(
                    "  close(epoch {}) at block {block}: frame {}",
                    epoch - 1,
                    measured.frame_gas
                );
            }
            if offset != 1 {
                continue;
            }
            if (1..=3).contains(&epoch) {
                fixture.commit_committee();
            }
            if epoch == 2 {
                // Arm the verdict sweep, so close(2) is the first judged close.
                fixture.govern(
                    IStaking::setProductionLivenessDisabledCall { value: false }.abi_encode(),
                );
                fixture.govern(IStaking::setMinVerdictDueBlocksCall { value: 1 }.abi_encode());
            }
            if epoch == 4 {
                // Unblock the stipend leg, so close(4) has four epochs to settle.
                fixture.govern(IStaking::setBlendReserveCall { value: OWNER }.abi_encode());
            }
        }
        assert_eq!(
            fixture.blocks_in_epoch(epoch),
            INTERVAL as u32,
            "epoch {epoch} must be fully recorded or its close skips the verdicts"
        );
    }

    let close_block = first_block(5);
    let pulls_before = fixture.committed_pulls();
    let worst = fixture.record(close_block);
    let pulls_in_close = fixture.committed_pulls() - pulls_before;
    println!(
        "  close(epoch 4) WORST at block {close_block}: frame {} / tx {} / post-refund {}",
        worst.frame_gas, worst.tx_gas, worst.tx_gas_after_refund
    );
    println!("  per-block baseline, no close: frame {baseline}");
    println!(
        "  whole close against the 30M system-call budget: {:.1}%",
        worst.frame_gas as f64 / 30_000_000.0 * 100.0
    );

    // Proof that the worst case really happened: two exclusions expired and were
    // released while two more were stamped, and four epochs settled.
    assert_eq!(fixture.pending_exclusions().len(), 2);
    // COUNT the transfers, positively. Two weaker forms were tried and both are
    // vacuous in the direction that matters. `epoch_rewards(e) > 0` stopped
    // proving settlement when the close took over writing that ledger — it is an
    // accrual record now and fills in whether or not a token moves. And "no
    // `StipendSkipped` was emitted" is vacuously TRUE of a walk that settled
    // NOTHING, so a regression that advances the cursor early would empty this
    // frame of settlement work, LOWER the measured intercept, and read green
    // against a ceiling. The pull counter is the only assertion here that fails
    // in both directions.
    assert_eq!(
        pulls_in_close, MAX_SETTLE_CATCHUP,
        "the close must fund exactly MAX_SETTLE_CATCHUP epochs, or the figure \
         above is not the worst case being guarded"
    );
    assert_eq!(
        worst.events(StipendLegSkipped::SIGNATURE_HASH),
        0,
        "the payment frame was discarded"
    );
    assert_eq!(
        worst.events(EpochBlendRewardsCommitted::SIGNATURE_HASH),
        1,
        "a close accrues exactly the one epoch it closes"
    );
    for epoch in 0..5u64 {
        assert!(
            fixture.epoch_rewards(epoch) > U256::ZERO,
            "epoch {epoch} must have been accrued by its own close"
        );
    }
    worst.frame_gas
}

/// What the accrual costs inside the close, measured rather than decomposed.
///
/// Two fixtures identical in every respect but the configured stipend rate. At
/// zero the accrual returns at its `pot.is_zero()` arm and writes only the
/// closed-marker scalar; above zero it walks the committee, splits the pot and
/// writes a credit per member. The difference is the split.
///
/// One thing is deliberately not held constant: the payment skips in the zero arm
/// and transfers in the funded one. `path_a_stipend_leg_cost_at_max_catchup`
/// prices that whole leg at four epochs, so the confound is bounded by a quarter
/// of that figure — two orders of magnitude below what is being measured here.
///
/// Both epochs are reported on purpose. Epoch 0's per-validator snapshots already
/// exist, materialized by `initialize`, so its accrual is the credit writes
/// alone. Epoch 1's do not, so it also pays `touch_snapshot_at_or_before` per
/// member — the steady-state shape, and work that MOVED out of the settlement
/// leg rather than being new.
#[test]
fn path_a_close_accrual_cost() {
    println!("\n=== PATH A: the accrual the close carries ===");
    let with_split = closes_at_stipend_rate(TOKEN * U256::from(COMMITTEE));
    let marker_only = closes_at_stipend_rate(U256::ZERO);
    println!(
        "{:>8}  {:>18}  {:>18}  {:>14}",
        "epoch", "close with split", "close, marker only", "accrual"
    );
    let accruals: Vec<u64> = with_split
        .iter()
        .zip(&marker_only)
        .map(|(full, marker)| full - marker)
        .collect();
    for (epoch, accrual) in accruals.iter().enumerate() {
        println!(
            "{epoch:>8}  {:>18}  {:>18}  {accrual:>14}",
            with_split[epoch], marker_only[epoch]
        );
    }
    let steady_state = accruals[1];
    println!(
        "\nsteady-state accrual (epoch 1, snapshots not yet materialized): \
         {steady_state}"
    );
    println!(
        "against the 30M system-call budget: {:.1}%",
        steady_state as f64 / SYSTEM_CALL_BUDGET * 100.0
    );

    let mut guards = Guards::default();
    guards.at_most(
        "close accrual, steady state",
        steady_state as f64,
        PATH_A_ACCRUAL_MAX as f64,
    );
    guards.finish();
}

/// Frame gas of `close(0)` and `close(1)` with the stipend priced at `rate`.
///
/// The verdict tier is left at its seeded-off default, so neither close judges
/// and the two arms differ only where the rate makes them.
fn closes_at_stipend_rate(rate: U256) -> Vec<u64> {
    let mut fixture = fixture(COMMITTEE, true);
    fixture.govern(IStaking::setBlendStipendPerEpochCall { value: rate }.abi_encode());
    // Two to close plus the lookahead the commit insists on.
    for _ in 0..3 {
        fixture.commit_committee();
    }
    let mut closes = Vec::new();
    for block in first_block(0)..=first_block(2) {
        let measured = fixture.record(block);
        if block == first_block(1) || block == first_block(2) {
            closes.push(measured.frame_gas);
        }
    }
    closes
}

/// The stipend leg alone, through `settleEpochStipend`, which runs the identical
/// `settle_up_to`. This is what has to fit inside `STIPEND_FUEL_CAP` — 12M
/// gas-equivalent — when the close forwards it as a self-call.
#[test]
fn path_a_stipend_leg_cost_at_max_catchup() {
    let mut fixture = fixture(COMMITTEE, false);
    println!("\n=== PATH A: stipend leg (settle_up_to) at maximum catch-up ===");
    let stalled_baseline = fixture.committed_pulls();

    for _ in 0..3 {
        fixture.commit_committee();
    }
    for epoch in 0..5u64 {
        let start = first_block(epoch);
        for offset in 0..INTERVAL {
            fixture.record(start + offset);
            if offset == 1 && (1..=3).contains(&epoch) {
                fixture.commit_committee();
            }
        }
    }
    // The cursor is still at epoch 0: every close so far hit the unapproved
    // source and its self-call frame was discarded. That premise is ASSERTED
    // rather than left to this comment — if it ever stops holding,
    // `settle_up_to` early-returns, the measured frame collapses to nothing and
    // `STIPEND_LEG_MAX` passes on a no-op.
    //
    // Against the post-genesis baseline, not against zero: `initialize` pulls the
    // genesis stakes through the same `transferFrom` and the counter sees it.
    assert_eq!(
        fixture.committed_pulls(),
        stalled_baseline,
        "the stall did not hold, so the cursor is not at epoch 0 and there is no \
         maximum catch-up left to measure"
    );
    fixture.govern(IStaking::setBlendReserveCall { value: OWNER }.abi_encode());

    let measured = measure(
        &mut fixture.context,
        SYSTEM_CALLER,
        IStaking::settleEpochStipendCall { epoch: 3 }.abi_encode(),
    );
    println!(
        "settle 4 epochs x {COMMITTEE} members: frame {} / tx {} / post-refund {}",
        measured.frame_gas, measured.tx_gas, measured.tx_gas_after_refund
    );
    println!(
        "against STIPEND_FUEL_CAP ({STIPEND_CAP_GAS} gas-equivalent): {:.1}%",
        measured.frame_gas as f64 / STIPEND_CAP_GAS as f64 * 100.0
    );
    // Four real pulls, COUNTED. The leg is priced on what it does, and after the
    // split it can walk four epochs while paying for none of them — "no
    // `StipendSkipped`" would be vacuously true of a walk that settled nothing,
    // which is the shrink-to-nothing failure `STIPEND_LEG_MAX` cannot see.
    assert_eq!(
        fixture.committed_pulls() - stalled_baseline,
        MAX_SETTLE_CATCHUP,
        "the measured frame did not fund MAX_SETTLE_CATCHUP epochs"
    );
    for epoch in 0..4u64 {
        let credited = fixture.epoch_rewards(epoch);
        println!("  epoch {epoch} accrued rewards: {credited}");
        assert!(
            credited > U256::ZERO,
            "epoch {epoch} must have been accrued, or the leg had nothing to pay"
        );
    }

    let mut guards = Guards::default();
    guards.at_most(
        "stipend leg at MAX_SETTLE_CATCHUP",
        measured.frame_gas as f64,
        STIPEND_LEG_MAX as f64,
    );
    guards.finish();
}
