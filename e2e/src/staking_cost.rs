//! Worst-case execution cost of the two hot staking system-call paths.
//!
//! Self-contained: nothing outside this file is referenced by it except
//! `fluentbase-staking-abi`, which declares every call with a second caller once
//! for all of them; and nothing outside references it except the `mod`
//! declaration in `lib.rs`.
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
// Everything with a second caller outside `contracts/staking` — `initialize`,
// `recordProduction`, `commitEpochCommittee`, `blocksInEpoch`,
// `getEpochRewards`, the two governance setters, and the
// `EpochBlendRewardsCommitted` event — comes from the crate the contract derives
// its dispatch selectors from, so an ABI change to any of them is a compile
// error here rather than a revert against the real blob.
use fluentbase_staking_abi as staking_abi;
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
/// close: the leg shed 6_603_076 while the close took on 3_264_563 of accrual
/// (see `PATH_A_ACCRUAL_MAX`). The close did the work for ONE epoch where the leg
/// did it for four, which is where the net saving comes from — not from the work
/// getting cheaper. Both the leg and the constants that priced it are gone as of
/// 2026-09-07; this paragraph is kept because the 3_552_595 above is only
/// intelligible with it.
///
/// Re-measured after FLU-1134 restructured the committee storage the close reads
/// (2026-08-17, same day): **3_992_457**, down a further 49_200. Small next to
/// the commit's 544_400, and it should be: the close READS the committee where
/// the commit WRITES it, and the layout change is a write-side saving. Recorded
/// because a measurement left stale is how a ceiling stops meaning anything, and
/// this file's own rule says so two constants up. The ceiling stays at 4_640_000
/// — 16% over, which is the headroom this file uses.
///
/// Re-measured 2026-09-07 after the stipend payment left the close entirely:
/// **3_954_277**, down 59_980 from the 4_014_257 this suite measured the same
/// day before the change. That is the fuel-capped self-call and the four-epoch
/// settlement walk leaving, less the two `static_call`s the close now makes to
/// read the reserve — a net saving of 1.5%, which is small because the leg was
/// already only 0.4% of its own cap. Ceiling left at 4_640_000: it is 17% over
/// the new figure, still inside the band this file uses, and re-pinning for a
/// 1.5% move would spend more attention than it buys.
const PATH_A_INTERCEPT_MAX: f64 = 4_640_000.0;
/// Measured 2026-08-17: 4_600 gas per roster entry, unchanged from 2026-08-06.
///
/// Left at its existing value deliberately: the roster walk is `release_expired`
/// and the exclusion sweep, neither of which the stipend split touches, and the
/// measurement confirms it to the digit. 5_300 is already ~15% over it.
const PATH_A_SLOPE_MAX: f64 = 5_300.0;
/// Measured 2026-08-17: roster 5_654 after FLU-1134 (5_643 before it, on the same
/// day; 4_871 on 2026-08-06).
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
///
/// Re-measured 2026-09-07 after the close started reading the BLEND reserve
/// before it prices an epoch: **3_211_372**, down 3_591. Two `static_call`s and
/// their ABI encoding, measured rather than assumed; the accrual is otherwise
/// untouched by the stipend split.
///
/// Re-measured after FLU-1134 (2026-08-17): **3_214_963**, down 49_600. Same
/// cause and the same size as the intercept's fall — it IS the intercept's fall,
/// since the accrual is where the close reads the committee.
const PATH_A_ACCRUAL_MAX: u64 = 3_750_000;

/// Measured 2026-08-17: **2_030_493**, down 544_400 from 2_574_893 on
/// 2026-08-06 — a fall of 21%.
///
/// The whole of it comes from FLU-1134's layout change, and the direction is the
/// opposite of the last two entries here. It used to rise: +143k when the
/// committee and its weights merged into one vector of pairs, bought for safety.
/// Splitting them again — but by LIFETIME rather than back into parallel vectors
/// — pays that back several times over. Membership is a record appended only
/// when the committee actually changes, so an unchanged epoch writes no member
/// slots at all; the weights go into a fixed ring where two `uint112` and their
/// epoch stamp share one 32-byte slot instead of a member costing two.
///
/// The plan that authorised this predicted ~445k at the cap. The measurement is
/// 544_400, so the estimate was ~18% low — recorded rather than quietly
/// corrected, because the estimate is what the decision was made against.
///
/// Pinned DOWN to the measurement with the ~15% headroom this file uses
/// elsewhere, not left at the old ceiling. A ceiling that no longer binds
/// records nothing.
const COMMIT_INTERCEPT_MAX: f64 = 2_340_000.0;
/// Re-measured 2026-09-07: **17_100** gas per active-set entry, up 4_300 from
/// 12_800 on 2026-08-06. The cost REGRESSED; the threshold had not aged.
///
/// The rise is bought, not accidental. The eligibility filter moved BEFORE the
/// stake cut, so the consensus-key read — `peer_pubkey` and its activation epoch,
/// two slots — is now paid for every candidate instead of only for the `cap`
/// that were seated. Ranking still walks the whole set as it always did; what
/// changed is how many of them get asked whether they could take a seat. That is
/// the price of the ordering itself: filtering after the cut spent a seat on a
/// validator that could not take it and did not pass it on, which is the defect
/// the reorder removes.
///
/// The alternative that would win it back is ranking first and then walking DOWN
/// the ranked list taking eligible members until `cap` is filled — identical
/// output, key read paid only down to the cut. It needs a selection that can
/// yield more than `cap` on demand, which `top_k_by_stake_at` cannot, so it is
/// not done here.
///
/// Pinned UP to the measurement with the ~15% headroom this file uses elsewhere.
const COMMIT_SLOPE_MAX: f64 = 19_700.0;
/// The binding limit of the three — see the minimum rule above.
///
/// Re-measured 2026-09-07: roster **1_648**, down from 2_185 on 2026-08-17, for
/// the slope above. Pinned DOWN by the same ~12% this file used when it last
/// moved a floor, NOT left at 1_920 where it would fail the very measurement it
/// is being set against.
///
/// Worth its own line: 1_648 is thirty-two times `MAX_ACTIVE_VALIDATORS_LENGTH`
/// (51, `consts.rs`), the largest active set the contract will accept. This guard
/// is a canary for the SHAPE of the cost, not a limit anything can reach.
const COMMIT_ROSTER_FLOOR: f64 = 1_450.0;

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

// The two handlers this file is the ONLY place outside `contracts/staking` to
// encode; under the rule in `fluentbase-staking-abi`'s header they stay here.
// Everything else this file calls comes from the shared crate.
sol! {
    interface IStaking {
        function pendingExclusions() external view returns (address[]);
        function setMinVerdictDueBlocks(uint32 value) external;
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

/// ERC-20 stand-in for the BLEND token, in the two shapes the contract uses it.
///
/// `transferFrom` succeeds for `OWNER` and reverts for anybody else — an address
/// that has not approved the staking contract is exactly an address whose
/// `transferFrom` reverts. `balanceOf` and `allowance`, which the epoch close
/// reads before it prices an epoch, both answer 2^80-1: far above any pot this
/// file configures, so the close is never forfeited for want of funding and the
/// measured frame is the full accrual. Every other selector returns `1`. No
/// balances are tracked; this measures gas, not solvency.
///
/// It DOES count successful pulls, in storage slot zero, and that counter is the
/// only positive evidence in this file that a measured frame moved money — which
/// after the stipend split is evidence that it must NOT. Read it with
/// `committed_pulls`.
fn counting_token() -> Vec<u8> {
    hex!(
        // sel = calldata[0..4]
        "60003560e01c"
        // sel == transferFrom(address,address,uint256) ? -> 0x42
        "80" "6323b872dd" "14" "6042" "57"
        // sel == balanceOf(address) ? -> 0x2e
        "80" "6370a08231" "14" "602e" "57"
        // sel == allowance(address,address) ? -> 0x2e
        "80" "63dd62ed3e" "14" "602e" "57"
        // anything else: return true
        "600160005260206000f3"
        // 0x2e: return 2^80 - 1
        "5b" "69ffffffffffffffffffff" "60005260206000f3"
        // 0x42: is the `from` argument OWNER?
        "5b" "60043573"
        "1111111111111111111111111111111111111111"
        "14" "6064" "57"
        // no: revert, which is what an unapproved source looks like
        "60006000fd"
        // 0x64: sstore(0, sload(0) + 1), then return true
        "5b" "600054" "600101" "600055" "600160005260206000f3"
    )
    .to_vec()
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

/// An address that has not approved the staking contract.
///
/// It no longer changes what a close does — the mock token answers the close's
/// two reserve reads generously whatever address they name, so `fixture`'s
/// `source_is_approved` now only decides which address is CONFIGURED. Kept
/// because that configuration is still the production-realistic one for a chain
/// whose treasury has not approved yet.
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

/// Real keys, because `initialize` now runs a real pairing over every one of
/// them. See `crate::bls_vectors`.
fn bls_pubkey(index: usize) -> Bytes {
    Bytes::copy_from_slice(crate::bls_vectors::pubkey(index))
}

fn bls_pop(index: usize) -> Bytes {
    Bytes::copy_from_slice(crate::bls_vectors::pop(index))
}

struct Fixture {
    context: EvmTestingContext,
    validators: Vec<Address>,
    token: Address,
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
            staking_abi::commitEpochCommitteeCall {}.abi_encode(),
        )
    }

    fn record(&mut self, block: u64) -> Measured {
        set_block(&self.context, block);
        measure(
            &mut self.context,
            SYSTEM_CALLER,
            staking_abi::recordProductionCall { leaderIndex: 0 }.abi_encode(),
        )
    }

    fn govern(&mut self, calldata: Vec<u8>) {
        call(&mut self.context, GENESIS_GOVERNANCE, calldata);
    }

    fn blocks_in_epoch(&mut self, epoch: u64) -> u32 {
        let output = call(
            &mut self.context,
            OWNER,
            staking_abi::blocksInEpochCall { epoch }.abi_encode(),
        );
        staking_abi::blocksInEpochCall::abi_decode_returns(&output).unwrap()
    }

    fn epoch_rewards(&mut self, epoch: u64) -> U256 {
        let output = call(
            &mut self.context,
            OWNER,
            staking_abi::getEpochRewardsCall { epoch }.abi_encode(),
        );
        staking_abi::getEpochRewardsCall::abi_decode_returns(&output).unwrap()
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

/// Genesis with `roster` active, equally staked, key-carrying validators.
///
/// They are seeded through `initialize`, the only path that makes a validator
/// Active and selection-visible from epoch 0 with no warm-up delay, so the
/// committee can be committed immediately.
fn fixture(roster: usize, source_is_approved: bool) -> Fixture {
    let mut context = EvmTestingContext::default().with_full_genesis();
    // Pinned, not inherited: the proof of possession each validator below carries
    // was signed under this chain id, and a mismatch would fail the whole genesis
    // with `InvalidProofOfPossession`.
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    set_block(&context, ACTIVATION - 1);

    let token = deploy_runtime(&mut context, &counting_token());

    let validators: Vec<Address> = (0..roster).map(validator_address).collect();
    let calldata = staking_abi::initializeCall {
        initialStakeOwner: OWNER,
        validators: validators.clone(),
        initialStakes: vec![TOKEN * U256::from(10); roster],
        blsPubkeysUncompressed: (0..roster).map(bls_pubkey).collect(),
        blsPopsUncompressed: (0..roster).map(bls_pop).collect(),
        peerPubkeys: (0..roster).map(peer_pubkey).collect(),
        commissionRate: 0,
        stakingToken: token,
        activeValidatorsLength: COMMITTEE as u32,
        epochBlockInterval: INTERVAL as u32,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: ACTIVATION,
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
        staking_abi::setBlendStipendPerEpochCall {
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

/// First block of `epoch`.
fn first_block(epoch: u64) -> u64 {
    ACTIVATION + epoch * INTERVAL
}

// ---------------------------------------------------------------------------
// Path B: the committee commit the node makes at every epoch boundary.
// ---------------------------------------------------------------------------

/// The READ half of this path is gone. Until 2026-09-07 the node re-derived the
/// committee off `getValidatorsWithKeysAt(epoch)` and this test measured that
/// view beside the commit that froze the same selection; the view, the per-epoch
/// selection roster it walked and the epoch-addressed cap it read were all
/// deleted when the contract dropped to ONE selection algorithm. `VIEW_*` guards
/// went with them. The commit is the whole path now, and it is the half that was
/// always load-bearing: it runs as a pre-execution system call against the 30M
/// budget, where the view ran as an ordinary `eth_call` with no budget at all.
///
/// The `roster` axis is now the size of the ACTIVE VALIDATOR SET, which is what
/// the commit walks. The name is kept because the fixture builds it the same way
/// and the constants are pinned against the old measurements.
#[test]
fn path_b_committee_commit_cost_by_roster_size() {
    println!("\n=== PATH B: commitEpochCommittee(), by active-set size ===");
    println!("{:>8}  {:>16}", "roster", "commit frame gas");
    let mut commit_points: Vec<(usize, u64)> = Vec::new();
    for roster in [COMMITTEE, 100, 200, 400] {
        let mut fixture = fixture(roster, true);
        let commit = fixture.commit_committee();
        println!("{roster:>8}  {:>16}", commit.frame_gas);
        commit_points.push((roster, commit.frame_gas));
    }
    let mut guards = Guards::default();

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
/// releases, all in one close.
///
/// The schedule:
///   epochs 0..=1  liveness tier off, so nothing is judged.
///   epoch 2       tier armed mid-epoch.
///   close(2)      50 first-time failures trip the correlation guard: verdicts
///                 are stamped, no exclusion is applied.
///   close(3)      the same 50 fail again as repeats, the guard stands down,
///                 `stamp` applies two exclusions.
///   close(4)      WORST. Judges 51 again, releases the two exclusions that
///                 expire, applies two more, and accrues the epoch it closes.
///
/// The settlement catch-up that used to be the fourth term here is gone with the
/// payment leg: the close reads the reserve and assigns credits, and no token
/// moves until somebody claims.
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
                    staking_abi::setProductionLivenessDisabledCall { value: false }.abi_encode(),
                );
                fixture.govern(IStaking::setMinVerdictDueBlocksCall { value: 1 }.abi_encode());
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
    // released while two more were stamped.
    assert_eq!(fixture.pending_exclusions().len(), 2);
    // COUNT the transfers, positively — and the count that proves the close is
    // doing its job is now ZERO. The close reads the reserve and writes credits;
    // a regression that pulled the pot onto this contract would show up here and
    // nowhere else, because `epoch_rewards` fills in either way.
    assert_eq!(
        pulls_in_close, 0,
        "the close must move no money: the reserve pays each claim directly"
    );
    assert_eq!(
        worst.events(staking_abi::EpochBlendRewardsCommitted::SIGNATURE_HASH),
        1,
        "a close accrues exactly the one epoch it closes"
    );
    // Every epoch accrued, including the ones that closed before any of this —
    // the accrual is unconditional on funding here because the mock token
    // answers both reserve reads far above the pot. A forfeited epoch would
    // shrink the measured frame and read green against the ceiling.
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
/// zero the accrual returns at its `pot.is_zero()` arm before it reads anything;
/// above zero it asks the reserve what it can cover, walks the committee, splits
/// the pot and writes a credit per member. The difference is that whole tail.
///
/// Nothing moves money in either arm any more, so the two are cleanly comparable
/// — the confound the old payment leg introduced here is gone with it.
///
/// Both epochs are reported on purpose. Epoch 0's per-validator snapshots already
/// exist, materialized by `initialize`, so its accrual is the credit writes
/// alone. Epoch 1's do not, so it also pays `touch_snapshot_at_or_before` per
/// member — the steady-state shape.
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
    fixture.govern(staking_abi::setBlendStipendPerEpochCall { value: rate }.abi_encode());
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
