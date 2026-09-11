//! Where the chain first holds `committee[E]` — the one fact the stand's
//! `FakeStaking` cannot be evidence for.
//!
//! Every other property the committee module is tested on is a property of the
//! READING code: which hash it reads at, how many calls it makes, whether two
//! nodes agree. `commit_height(E)` is not — it is a property of the CONTRACT,
//! and the stand's fake reproduces it from a rule written down beside the fake
//! (`testbed/fakes.rs`, `FakeStaking::committed_at`) rather than read off the
//! blob. A fake that is wrong there is wrong in the same direction in every
//! test that uses it, so the node's own
//! `committee::Geometry::commit_height` — `start(E − MAX_COMMITTEE_LOOKAHEAD_EPOCHS)`,
//! with the first three epochs at height 1 — is pinned HERE, against the real
//! rWasm predeploy.
//!
//! What the pin is: the SAME call, refused at `start(c) − 1` and accepted at
//! `start(c)`, for the epoch `c + 2` that boundary is about.
//!
//! What it is NOT, and the line matters: the `e <= 2 => 1` arm of
//! `commit_height` is a fact about the NODE, not about the contract. The
//! contract's horizon is `target <= current_epoch(block) + 2`
//! (`contracts/staking/src/consensus.rs:536-545`), and `current_epoch` clamps
//! to 0 on every pre-activation block through the `saturating_sub` in
//! `epoch_at_block` (`crates/types/src/staking_protocol.rs:169-178`) — so
//! blocks 0, 1 and every block below `dposActivationBlock` accept exactly the
//! same three targets. The `1` is where the NODE first gets to make the call:
//! genesis is built, not executed, and the bootstrap issues one
//! `commitEpochCommittee` for epoch 0 alone
//! (`devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:399-408`), so
//! the first EXECUTED block — height 1 — is the first pre-execution drain, and
//! that is what `committee/mod.rs::Geometry::commit_height` encodes. This file
//! pins the contract half: that the contract does not get in the way, and that
//! it stops at 3.

use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolError};
use fluentbase_sdk::{
    address,
    universal_token::{ApproveCommand, InitialSettings, UniversalTokenCommand},
    Address, Bytes, B256, GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_staking_abi as staking_abi;
use fluentbase_testing::EvmTestingContext;

const OWNER: Address = Address::repeat_byte(0x11);
const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

/// `MIN_COMMITTEE_LENGTH` — the floor `commitEpochCommittee` reverts below, so
/// the smallest roster this pin can run on.
const ROSTER: usize = 4;
const INTERVAL: u64 = 100;
/// A multiple of [`INTERVAL`], which `validate_initialization` enforces.
const ACTIVATION: u64 = 10 * INTERVAL;
/// The epoch whose `start` the boundary half of the pin sits on. Any `c >= 1`
/// works; 3 is far enough from genesis that the first-three-epochs exception
/// below cannot be what makes it pass.
const C: u64 = 3;

// The revert `commitEpochCommittee` raises for a target above the horizon. It
// is declared here and not in `fluentbase-staking-abi` because the node never
// decodes it: the call is a pre-execution system call, and a revert there is a
// block-execution error rather than a value anything reads. Declaring it makes
// the test pin WHICH refusal fired — a commit has several ways to revert, and
// the committee floor is cheaper to hit by accident than the horizon.
sol! {
    error EpochNotYetCommittable(uint64 target, uint64 current);
}

/// First block of `epoch`.
fn start(epoch: u64) -> u64 {
    ACTIVATION + epoch * INTERVAL
}

fn call(context: &mut EvmTestingContext, caller: Address, input: Vec<u8>) -> Vec<u8> {
    let result = context.call_evm_tx(
        caller,
        GENESIS_STAKING,
        input.into(),
        Some(20_000_000),
        None,
    );
    assert!(result.is_success(), "call failed: {result:?}");
    result.output().cloned().unwrap_or_default().to_vec()
}

fn validator(index: usize) -> Address {
    let mut bytes = [0u8; 20];
    bytes[0] = 0xa0;
    bytes[19] = index as u8;
    Address::from(bytes)
}

/// Strictly ascending in `index` — the order `commitEpochCommittee` sorts the
/// committee into before storing it.
fn peer_pubkey(index: usize) -> B256 {
    B256::with_last_byte(index as u8 + 1)
}

/// `commitEpochCommittee()` from the system caller: `Ok(())` when it committed,
/// `Err((target, current))` when it refused the target as above the horizon.
///
/// A revert that is NOT the horizon refusal panics rather than being folded
/// into `Err`: the whole point of the pin is which epoch the contract will not
/// commit yet, and a committee-floor or system-caller revert answering the same
/// `Err` would make every assertion below pass for the wrong reason.
fn commit(context: &mut EvmTestingContext) -> Result<(), (u64, u64)> {
    let input = staking_abi::commitEpochCommitteeCall {}.abi_encode();
    let result = context.call_evm_tx(
        SYSTEM_CALLER,
        GENESIS_STAKING,
        input.into(),
        Some(20_000_000),
        None,
    );
    if result.is_success() {
        return Ok(());
    }
    let output = result.output().cloned().unwrap_or_default();
    assert_eq!(
        output.get(..4),
        Some(&EpochNotYetCommittable::SELECTOR[..]),
        "commitEpochCommittee reverted, but not for the reason under test: {output:?}"
    );
    let refusal = EpochNotYetCommittable::abi_decode(&output).expect("the horizon refusal decodes");
    Err((refusal.target, refusal.current))
}

fn next_epoch_to_commit(context: &mut EvmTestingContext) -> u64 {
    let output = call(
        context,
        OWNER,
        staking_abi::nextEpochToCommitCall {}.abi_encode(),
    );
    staking_abi::nextEpochToCommitCall::abi_decode_returns(&output).unwrap()
}

/// The addresses of the frozen `epoch` committee — EMPTY for an epoch the
/// contract has not committed, which is the answer the node's reader turns into
/// `CommitteeError::NotReadable`.
fn committee(context: &mut EvmTestingContext, epoch: u64) -> Vec<Address> {
    let output = call(
        context,
        OWNER,
        staking_abi::getEpochCommitteeWithStakesCall { epoch }.abi_encode(),
    );
    staking_abi::getEpochCommitteeWithStakesCall::abi_decode_returns(&output)
        .unwrap()
        .addrs
}

/// Genesis with `ROSTER` equally staked, key-carrying validators, initialized
/// pre-activation so the current epoch is clamped to zero.
fn fixture() -> EvmTestingContext {
    let mut context = EvmTestingContext::default().with_full_genesis();
    // Pinned, not inherited: each proof of possession below was signed under
    // this chain id, and a mismatch fails the whole genesis.
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    context = context.with_block_number(ACTIVATION - 1);
    let token = context.deploy_evm_tx(
        OWNER,
        InitialSettings {
            token_name: "Blend".into(),
            token_symbol: "BLEND".into(),
            decimals: 18,
            initial_supply: TOKEN * U256::from(1_000),
            minter: OWNER,
            pauser: Address::ZERO,
            wrapped: None,
        }
        .encode_with_prefix(),
    );
    let mut approve = Vec::new();
    ApproveCommand {
        spender: GENESIS_STAKING,
        amount: TOKEN * U256::from(ROSTER),
    }
    .encode_for_send(&mut approve);
    let result = context.call_evm_tx(OWNER, token, approve.into(), Some(20_000_000), None);
    assert!(result.is_success(), "approve failed: {result:?}");

    call(
        &mut context,
        GENESIS_GOVERNANCE,
        staking_abi::initializeCall {
            initialStakeOwner: OWNER,
            validators: (0..ROSTER).map(validator).collect(),
            initialStakes: vec![TOKEN; ROSTER],
            blsPubkeysUncompressed: (0..ROSTER)
                .map(|i| Bytes::copy_from_slice(crate::bls_vectors::pubkey(i)))
                .collect(),
            blsPopsUncompressed: (0..ROSTER)
                .map(|i| Bytes::copy_from_slice(crate::bls_vectors::pop(i)))
                .collect(),
            peerPubkeys: (0..ROSTER).map(peer_pubkey).collect(),
            commissionRate: 0,
            stakingToken: token,
            activeValidatorsLength: ROSTER as u32,
            epochBlockInterval: INTERVAL as u32,
            undelegatePeriod: 7,
            minValidatorStakeAmount: TOKEN,
            minStakingAmount: TOKEN,
            dposActivationBlock: ACTIVATION,
            minUndelegateBlocks: U256::ZERO,
            blendReserve: Address::repeat_byte(0x66),
        }
        .abi_encode(),
    );
    context
}

/// The pre-activation walk: from a cursor at 0, the contract commits epochs 0,
/// 1 and 2 at `block` and then refuses 3 naming `current = 0`.
///
/// Takes the block because the two blocks it is called with — 0 and 1 — are
/// exactly the pair the node's `commit_height` distinguishes and the contract
/// does not. Asserting both is what keeps the doc honest: the walk is the
/// contract's horizon at a clamped `current`, and nothing about which block the
/// node first executes.
fn walk_the_first_three_epochs(block: u64) {
    let mut context = fixture().with_block_number(block);
    assert_eq!(
        next_epoch_to_commit(&mut context),
        0,
        "initialize left the commit cursor somewhere other than epoch 0"
    );
    for target in 0..=2u64 {
        assert_eq!(
            commit(&mut context),
            Ok(()),
            "epoch {target} is not committable at block {block}"
        );
        assert_eq!(next_epoch_to_commit(&mut context), target + 1);
        assert!(
            !committee(&mut context, target).is_empty(),
            "epoch {target} committed but reads back empty"
        );
    }
    assert_eq!(
        commit(&mut context),
        Err((3, 0)),
        "epoch 3 is committable at block {block}, so the horizon is not two epochs at a \
         clamped current epoch"
    );
    assert!(
        committee(&mut context, 3).is_empty(),
        "the refused epoch 3 has a committee anyway"
    );
}

/// `committee[E]` is first committable in the state of `start(E − 2)`, and
/// below the activation the contract holds its horizon open at `current = 0`.
///
/// The two halves, against the real blob:
///
/// * **Pre-activation horizon.** At block 0 AND at block 1 — both
///   pre-activation, so `current_epoch` clamps to 0 in both — the cursor walks
///   0 → 1 → 2 and then refuses 3. The two blocks answer identically, which is
///   the point: `start(E − 2)` would put epochs 1 and 2 at
///   `start(0) = ACTIVATION`, and the contract instead accepts them from the
///   very bottom of the chain. WHICH of those blocks actually issues the call
///   is the node's business — genesis is not executed, so it is height 1, which
///   is what `Geometry::commit_height` reads as `e <= 2 => 1`. Nothing in this
///   test is evidence for that half, and it must not be read as such (module
///   doc above).
/// * **Boundary.** With the cursor standing at `C + 2`, the SAME call is
///   refused at `start(C) − 1` and accepted at `start(C)`. That is the
///   equality, not an inequality: one block earlier is a revert naming
///   `(target C+2, current C−1)`, and one block later the committee is there.
///
/// Falsifier: the cursor reaching `C + 3` at `start(C)` (the horizon is wider
/// than two epochs); the refusal at `start(C) − 1` naming another target or
/// another current epoch; a non-empty `committee[C+2]` before the accepted
/// call, or an empty one after; either pre-activation walk stopping anywhere
/// but at target 3, or the two walks disagreeing.
#[test]
fn a_committee_is_first_committable_two_epochs_before_its_own_first_block() {
    // PRE-ACTIVATION HORIZON: the same three targets from the bottom of the
    // chain, whichever pre-activation block asks.
    walk_the_first_three_epochs(0);
    walk_the_first_three_epochs(1);

    // The boundary half runs on its own fixture, walked up from block 1.
    let mut context = fixture().with_block_number(1);
    for target in 0..=2u64 {
        assert_eq!(
            commit(&mut context),
            Ok(()),
            "epoch {target} is not committable at block 1"
        );
    }

    // Walk the cursor up to `C + 2` inside epoch `C - 1`, which is where the
    // horizon stops it: `current = C - 1`, so `C + 1` is the last acceptable
    // target.
    context = context.with_block_number(start(C) - 1);
    loop {
        match commit(&mut context) {
            Ok(()) => continue,
            Err(refused) => {
                assert_eq!(
                    refused,
                    (C + 2, C - 1),
                    "the last block of epoch {} refused the wrong target",
                    C - 1
                );
                break;
            }
        }
    }
    assert_eq!(next_epoch_to_commit(&mut context), C + 2);
    assert!(
        !committee(&mut context, C + 1).is_empty(),
        "epoch {} is not committed at the last block of epoch {}",
        C + 1,
        C - 1
    );
    assert!(
        committee(&mut context, C + 2).is_empty(),
        "epoch {} is committed one block before start({C})",
        C + 2
    );

    // BOUNDARY: one block later — the FIRST block of epoch `C` — the same call
    // goes through, and only that one.
    context = context.with_block_number(start(C));
    assert_eq!(
        commit(&mut context),
        Ok(()),
        "epoch {} is not committable at start({C}) = {}",
        C + 2,
        start(C)
    );
    assert!(
        !committee(&mut context, C + 2).is_empty(),
        "epoch {} committed at start({C}) but reads back empty",
        C + 2
    );
    assert_eq!(
        commit(&mut context),
        Err((C + 3, C)),
        "epoch {} is committable at start({C}), so the horizon moved",
        C + 3
    );
    assert!(
        committee(&mut context, C + 3).is_empty(),
        "the refused epoch {} has a committee anyway",
        C + 3
    );
}
