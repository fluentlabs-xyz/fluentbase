//! The fakes here are deliberately not `testbed::fakes::FakeStaking`: the properties under
//! test are properties of the caller, so the fake has to be able to break each of them —
//! branch on the read hash, withhold weights, answer empty, fail, and count every staticcall.

use super::*;
use crate::beacon::CommitteeReads as _;
use commonware_codec::DecodeExt as _;
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
use commonware_math::algebra::Random as _;
use fluentbase_bls::keys::ValidatorBlsKeypair;
use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
use rand_08::rngs::StdRng;
use rand_core::SeedableRng as _;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Barrier, Mutex,
    },
};

const INTERVAL: u64 = 32;

fn geometry() -> Geometry {
    Geometry::new(0, INTERVAL).expect("non-zero interval")
}

/// `n` validators in the contract's order (ascending raw peer pubkey): the only order a real
/// snapshot can arrive in.
fn validators(n: usize) -> Vec<ValidatorWithKeys> {
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    let mut out: Vec<ValidatorWithKeys> = (0..n)
        .map(|i| {
            let peer = Ed25519PrivateKey::random(&mut rng).public_key();
            let bls = ValidatorBlsKeypair::generate(&mut rng);
            ValidatorWithKeys {
                address: alloy_primitives::Address::with_last_byte(i as u8 + 1),
                keys: ConsensusKeys {
                    bls_pubkey: BlsPubkey::decode(bls.public_bytes().as_slice()).unwrap(),
                    peer_pubkey: peer,
                    activation_epoch: 0,
                },
                tombstoned: false,
            }
        })
        .collect();
    out.sort_by(|a, b| a.keys.peer_pubkey.cmp(&b.keys.peer_pubkey));
    out
}

fn hash(byte: u8) -> B256 {
    B256::repeat_byte(byte)
}

/// A movable ordering-finalized cursor plus a movable state probe, kept independent because
/// the production anchor keeps those two apart.
struct FakeAnchor {
    height: AtomicU64,
    /// Height → executed hash; a height absent here is the `executed_state_hash` park.
    executed: Mutex<BTreeMap<u64, B256>>,
    fault: AtomicBool,
}

impl FakeAnchor {
    fn at(height: u64, hash: B256) -> Arc<Self> {
        Arc::new(Self {
            height: AtomicU64::new(height),
            executed: Mutex::new(BTreeMap::from([(height, hash)])),
            fault: AtomicBool::new(false),
        })
    }

    fn unexecuted(height: u64) -> Arc<Self> {
        Arc::new(Self {
            height: AtomicU64::new(height),
            executed: Mutex::new(BTreeMap::new()),
            fault: AtomicBool::new(false),
        })
    }

    fn advance(&self, height: u64, hash: B256) {
        self.executed.lock().unwrap().insert(height, hash);
        self.height.store(height, Ordering::Release);
    }
}

impl Anchor for FakeAnchor {
    fn height(&self) -> u64 {
        self.height.load(Ordering::Acquire)
    }

    fn executed_hash(&self, height: u64) -> Result<Option<B256>, ReadError> {
        if self.fault.load(Ordering::Acquire) {
            return Err(ReadError::Backend(format!("block_hash({height}) is None")));
        }
        Ok(self.executed.lock().unwrap().get(&height).copied())
    }
}

/// What the contract answers for one `(epoch, at)`.
#[derive(Clone)]
enum Answer {
    Committee(Vec<usize>),
    /// A committed committee whose weights are handed over verbatim — the only way to give the
    /// module a weight vector whose length does not match the member array.
    CommitteeWeighted(Vec<usize>, Vec<u128>),
    /// Committed, but the contract's weight ring has wrapped past this epoch.
    WeightsWrapped(Vec<usize>),
    Uncommitted,
    Failed(&'static str),
}

impl Answer {
    fn error(kind: &'static str) -> ReadError {
        match kind {
            "transient" => ReadError::StateNotMaterialized { hash: B256::ZERO },
            _ => ReadError::CallReverted("execution reverted".into()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Calls {
    snapshot: Vec<(u64, B256)>,
    qual: Vec<(u64, B256)>,
}

impl Calls {
    fn snapshots_of(&self, epoch: u64) -> usize {
        self.snapshot.iter().filter(|(e, _)| *e == epoch).count()
    }
    fn quals_of(&self, epoch: u64) -> usize {
        self.qual.iter().filter(|(e, _)| *e == epoch).count()
    }
}

type Plan = Box<dyn Fn(u64, B256, usize) -> Answer + Send + Sync>;

/// What `getDkgQual(epoch)` answers at `at`: a fallible leg of its own, asked only after the
/// snapshot call is paid for, and free to branch on the hash like the other leg.
type QualPlan = Box<dyn Fn(u64, B256) -> Result<bool, ReadError> + Send + Sync>;

/// The everyday bit: the contract writes it in the same call that commits the committee, so
/// in-window it is a function of the epoch alone.
fn qual_by_epoch() -> QualPlan {
    Box::new(|epoch, _| Ok(epoch % 2 == 1))
}

struct FakeReads {
    validators: Vec<ValidatorWithKeys>,
    /// `(epoch, at, snapshot reads of this epoch that came before)`.
    plan: Plan,
    /// Hash → block number, so a snapshot reports its height the way the real reader does.
    heights: BTreeMap<B256, u64>,
    qual: QualPlan,
    calls: Mutex<Calls>,
    /// Barrier held so both concurrent readers miss the map before either installs.
    gate: Option<Arc<Barrier>>,
}

impl FakeReads {
    fn new(plan: Plan, heights: BTreeMap<B256, u64>) -> Self {
        Self {
            validators: validators(6),
            plan,
            heights,
            qual: qual_by_epoch(),
            calls: Mutex::new(Calls::default()),
            gate: None,
        }
    }

    fn with_qual(mut self, qual: QualPlan) -> Self {
        self.qual = qual;
        self
    }

    fn calls(&self) -> Calls {
        self.calls.lock().unwrap().clone()
    }
}

impl EpochReads for FakeReads {
    fn epoch_committee_snapshot(
        &self,
        epoch: u64,
        at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        let nth = {
            let mut calls = self.calls.lock().unwrap();
            let nth = calls.snapshots_of(epoch);
            calls.snapshot.push((epoch, at));
            nth
        };
        let answer = (self.plan)(epoch, at, nth);
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        let (slots, weights) = match answer {
            Answer::Committee(slots) => {
                let w = Some(vec![7u128; slots.len()]);
                (slots, w)
            }
            Answer::CommitteeWeighted(slots, weights) => (slots, Some(weights)),
            Answer::WeightsWrapped(slots) => (slots, None),
            Answer::Uncommitted => (Vec::new(), Some(Vec::new())),
            Answer::Failed(kind) => return Err(Answer::error(kind)),
        };
        Ok(ValidatorSetSnapshot {
            block_hash: at,
            block_number: *self.heights.get(&at).unwrap_or(&0),
            epoch,
            validators: slots
                .into_iter()
                .map(|i| self.validators[i].clone())
                .collect(),
            weights,
        })
    }

    fn dkg_qual(&self, epoch: u64, at: B256) -> Result<bool, ReadError> {
        self.calls.lock().unwrap().qual.push((epoch, at));
        (self.qual)(epoch, at)
    }
}

fn four_members() -> Plan {
    Box::new(|_, _, _| Answer::Committee(vec![0, 1, 2, 3]))
}

/// A watch already carrying the frozen pair. Dropping the temporary sender is fine: `borrow()`
/// still reads the last value, and that is all the store does with it.
fn frozen_geometry_rx() -> crate::committee::GeometryRx {
    tokio::sync::watch::Sender::new(Some((0, INTERVAL))).subscribe()
}

/// A verify-only scheme over the record's own BLS projection, bound to no oracle: these tests
/// are about the map, and the two about verification strength build their own.
fn test_verifier() -> EpochVerifier {
    Arc::new(|record: &CommitteeRecord| {
        Some(fluentbase_bls::scheme::build_verifier(
            &fluentbase_bls::fluent_namespace(CHAIN_ID),
            record.bls.bimap.clone(),
            record.epoch,
            None,
        ))
    })
}

const CHAIN_ID: u64 = 1;

fn new_store(anchor: Arc<FakeAnchor>, reads: FakeReads) -> Arc<CommitteeStore<FakeReads>> {
    Arc::new(CommitteeStore::new(
        reads,
        anchor,
        frozen_geometry_rx(),
        test_verifier(),
    ))
}

#[test]
fn without_a_frozen_geometry_every_epoch_is_not_readable_without_a_single_read() {
    // A real node starts this way: the store is built beside the anchor, before the beacon
    // plane has frozen `(activation, interval)`, so there is no epoch arithmetic to read with.
    let (tx, rx) = tokio::sync::watch::channel(None);
    let anchor = FakeAnchor::at(400, hash(7));
    let store = Arc::new(CommitteeStore::new(
        FakeReads::new(four_members(), BTreeMap::from([(hash(7), 400u64)])),
        anchor,
        rx,
        test_verifier(),
    ));

    match store.committee(5) {
        Err(CommitteeError::NotReadable { epoch, ready_at }) => {
            assert_eq!(
                (epoch, ready_at),
                (5, 0),
                "no height can make this readable"
            );
        }
        other => panic!("expected NotReadable while the geometry is unfrozen, got {other:?}"),
    }
    assert_eq!(
        store.reads().calls(),
        Calls::default(),
        "an unfrozen geometry must not cost a staticcall"
    );
    let mut advances = store.anchor_advances();
    assert_eq!(*advances.borrow_and_update(), 400);
    store.anchor_advanced();
    assert!(
        !advances.has_changed().expect("sender alive"),
        "no geometry, no wake-up"
    );

    // The freeze lands: the same store answers, with no rebuild.
    tx.send_replace(Some((0, INTERVAL)));
    store
        .committee(5)
        .expect("readable once the geometry is frozen");
    store.anchor_advanced();
    assert!(
        advances.has_changed().expect("sender alive"),
        "the first advance after the freeze is a wake-up"
    );
    assert_eq!(*advances.borrow_and_update(), 400);
}

#[test]
fn an_epoch_below_its_commit_height_is_not_readable_without_a_single_read() {
    // Epochs 1 and 2 are first committed by the first executed block, height 1, not at genesis.
    let anchor = FakeAnchor::at(0, hash(1));
    let store = new_store(anchor, FakeReads::new(four_members(), BTreeMap::new()));

    for epoch in [1u64, 2] {
        match store.committee(epoch) {
            Err(CommitteeError::NotReadable { epoch: e, ready_at }) => {
                assert_eq!(e, epoch);
                assert_eq!(ready_at, 1, "epochs 1 and 2 are committed by block 1");
            }
            other => panic!("expected NotReadable for epoch {epoch}, got {other:?}"),
        }
    }
    assert_eq!(
        store.reads().calls(),
        Calls::default(),
        "the commit-height gate is arithmetic — it must not touch the EVM"
    );
    assert_eq!(geometry().commit_height(0), 0, "genesis commits epoch 0");
    assert_eq!(geometry().commit_height(5), geometry().start(3));
}

#[test]
fn an_anchor_whose_state_is_not_materialized_is_not_readable_without_a_single_read() {
    // Backfill: reth wrote the header but not the state, so the probe answers `Ok(None)` and
    // the module parks without a staticcall — the reason the anchor probes state at all.
    let anchor = FakeAnchor::unexecuted(320);
    let store = new_store(anchor, FakeReads::new(four_members(), BTreeMap::new()));

    assert!(matches!(
        store.committee(10),
        Err(CommitteeError::NotReadable { epoch: 10, .. })
    ));
    assert_eq!(store.reads().calls(), Calls::default());
}

#[test]
fn an_epoch_outside_the_window_is_refused_without_a_single_read_and_the_borders_are_inside() {
    let at = hash(3);
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor,
        FakeReads::new(four_members(), BTreeMap::from([(at, 325)])),
    );

    // Anchor 325 is epoch 10 ⇒ window [10 − SCHEME_RETENTION_EPOCHS, 10 + 2] = [2, 12].
    for outside in [1u64, 13] {
        match store.committee(outside) {
            Err(CommitteeError::OutOfWindow { epoch, lo, hi }) => {
                assert_eq!((epoch, lo, hi), (outside, 2, 12));
            }
            other => panic!("expected OutOfWindow for {outside}, got {other:?}"),
        }
    }
    assert_eq!(
        store.reads().calls(),
        Calls::default(),
        "the window is a predicate on the REQUEST — it must not touch the EVM"
    );

    // Both borders are inside, so the refusals above are the window and not an off-by-one.
    for border in [2u64, 12] {
        assert!(
            store.committee(border).is_ok(),
            "epoch {border} is on the window border and must be readable"
        );
    }
}

#[test]
fn the_first_read_is_one_snapshot_and_one_qual_at_one_hash_and_the_second_is_free() {
    let at = hash(4);
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor,
        FakeReads::new(four_members(), BTreeMap::from([(at, 325)])),
    );

    let first = store.committee(10).expect("epoch 10 is readable");
    let calls = store.reads().calls();
    assert_eq!(calls.snapshots_of(10), 1);
    assert_eq!(calls.quals_of(10), 1);
    assert_eq!(
        calls.snapshot, calls.qual,
        "the qual bit is the SECOND leg of ONE question — same epoch, same hash"
    );

    let second = store.committee(10).expect("cached");
    assert!(Arc::ptr_eq(&first, &second), "write-once: the same Arc");
    assert_eq!(
        store.reads().calls(),
        calls,
        "a cached epoch costs no staticcall"
    );
}

#[test]
fn two_anchors_on_different_branches_produce_the_same_record() {
    // The fake branches on the hash — epoch 7 answers a different committee per branch — so the
    // epoch-5 equality is a property of the module, not of a fake that cannot disagree.
    let (a, b) = (hash(0xAA), hash(0xBB));
    let heights = BTreeMap::from([(a, 200u64), (b, 260u64)]);

    // Anchor 200 is epoch 6 ⇒ window [0, 8]; anchor 260 is epoch 8 ⇒ [0, 10].
    let left = new_store(
        FakeAnchor::at(200, a),
        FakeReads::new(branching(a, b), heights.clone()).with_qual(branching_qual(a)),
    );
    let right = new_store(
        FakeAnchor::at(260, b),
        FakeReads::new(branching(a, b), heights).with_qual(branching_qual(a)),
    );

    let l = left.committee(5).expect("readable on the left branch");
    let r = right.committee(5).expect("readable on the right branch");
    assert!(
        l.same_value(&r),
        "one epoch, two anchors, two heights — one record"
    );
    assert_eq!(l.members, r.members);
    assert_eq!(l.weights, r.weights);
    assert_eq!(l.changed, r.changed);
    assert_ne!(
        l.snapshot, r.snapshot,
        "the two records really were read at different blocks"
    );

    let l7 = left.committee(7).expect("readable");
    let r7 = right.committee(7).expect("readable");
    assert!(
        !l7.same_value(&r7),
        "the fake must be ABLE to answer two different committees per branch, \
         or the equality above proves nothing"
    );
    assert_ne!(
        l7.changed, r7.changed,
        "and ABLE to answer two different qual bits per branch, or the bit equality \
         above proves nothing either"
    );
}

/// The qual leg of the same contract: only epoch 7 answers a different bit per branch.
fn branching_qual(a: B256) -> QualPlan {
    Box::new(move |epoch, at| match epoch {
        7 => Ok(at == a),
        _ => Ok(epoch % 2 == 1),
    })
}

/// A contract whose epoch-7 answer depends on the branch and whose epoch-5 answer does not.
fn branching(a: B256, b: B256) -> Plan {
    Box::new(move |epoch, at, _| match (epoch, at) {
        (7, x) if x == a => Answer::Committee(vec![0, 1, 2, 3]),
        (7, x) if x == b => Answer::Committee(vec![2, 3, 4, 5]),
        _ => Answer::Committee(vec![0, 1, 2, 3]),
    })
}

#[test]
fn a_second_answer_for_one_epoch_is_refused_and_the_first_record_stands() {
    // Two consumers can miss the map for one epoch and both read, because the lock is not held
    // across the blocking state read; the barrier makes that race deterministic.
    let at = hash(6);
    let gate = Arc::new(Barrier::new(2));
    let plan: Plan = Box::new(|epoch, _, nth| {
        if epoch == 10 && nth == 1 {
            Answer::Committee(vec![2, 3, 4, 5])
        } else {
            Answer::Committee(vec![0, 1, 2, 3])
        }
    });
    let mut reads = FakeReads::new(plan, BTreeMap::from([(at, 325u64)]));
    reads.gate = Some(gate);
    let store = new_store(FakeAnchor::at(325, at), reads);

    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                scope.spawn(move || store.committee(10))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let kept: Vec<_> = outcomes.iter().filter_map(|o| o.as_ref().ok()).collect();
    let refused: Vec<_> = outcomes.iter().filter_map(|o| o.as_ref().err()).collect();
    assert_eq!(kept.len(), 1, "exactly one of the two answers is installed");
    assert_eq!(refused.len(), 1, "the disagreeing one is refused");
    assert!(
        !refused[0].is_transient(),
        "a contract fork is permanent, not a retry"
    );

    // The epoch is poisoned from here on: the chain stated two different committees for one
    // epoch, and serving whichever read won the race would serve one of two guesses.
    let refusal = store
        .committee(10)
        .expect_err("a forked epoch answers the refusal, not a record");
    assert!(
        refusal.is_contract_impossible(),
        "a contract fork is the impossible class: {refusal:?}"
    );
    assert_eq!(store.poisoned_epochs(), vec![10]);
    assert_eq!(
        store.reads().calls().snapshots_of(10),
        2,
        "no third read: the refusal came from the poisoned slot"
    );
    assert!(
        store.scheme(10).is_none(),
        "a forked epoch keeps no certificate scheme either"
    );
}

/// `weights: None` inside the window is the impossible class, and an impossible answer is
/// memoised: the epoch is poisoned, so the same refusal comes back without a second staticcall.
#[test]
fn absent_weights_inside_the_window_are_permanent_and_poison_the_epoch() {
    let at = hash(7);
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(
            Box::new(|_, _, _| Answer::WeightsWrapped(vec![0, 1, 2, 3])),
            BTreeMap::from([(at, 325u64)]),
        ),
    );

    let err = store
        .committee(10)
        .expect_err("weights: None inside the window");
    assert!(matches!(err, CommitteeError::Read(_)));
    assert!(
        !err.is_transient(),
        "the ring cannot have wrapped past an epoch in the window — this is a fork, not a retry"
    );

    assert!(
        err.is_contract_impossible(),
        "the ring cannot have wrapped past an in-window epoch — the contract answered something \
         impossible: {err:?}"
    );
    assert_eq!(store.reads().calls().snapshots_of(10), 1);

    let again = store.committee(10).expect_err("the epoch stays refused");
    assert_eq!(again.to_string(), err.to_string());
    assert_eq!(
        store.reads().calls().snapshots_of(10),
        1,
        "the poisoned slot spent another staticcall"
    );
    assert_eq!(store.poisoned_epochs(), vec![10]);
    assert!(
        store.cached_epochs().is_empty(),
        "a poisoned epoch holds no record"
    );
    assert!(store.scheme(10).is_none(), "and no scheme");
}

#[test]
fn a_transient_read_error_caches_nothing_and_is_retried() {
    let at = hash(8);
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(
            Box::new(|_, _, nth| {
                if nth == 0 {
                    Answer::Failed("transient")
                } else {
                    Answer::Committee(vec![0, 1, 2, 3])
                }
            }),
            BTreeMap::from([(at, 325u64)]),
        ),
    );

    let err = store.committee(10).expect_err("state not materialized");
    assert!(matches!(err, CommitteeError::Read(_)));
    assert!(
        err.is_transient(),
        "StateNotMaterialized re-materializes itself"
    );
    assert_eq!(store.reads().calls().snapshots_of(10), 1);

    store.committee(10).expect("the retry succeeds");
    assert_eq!(
        store.reads().calls().snapshots_of(10),
        2,
        "the failed read cached nothing, so the retry really went to the contract"
    );

    // The other half: a revert is the contract speaking, and it will say the same thing again.
    let reverting = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(
            Box::new(|_, _, _| Answer::Failed("permanent")),
            BTreeMap::from([(at, 325u64)]),
        ),
    );
    let err = reverting.committee(10).expect_err("the call reverts");
    assert!(matches!(err, CommitteeError::Read(_)));
    assert!(!err.is_transient(), "CallReverted is not a retry");
    assert!(reverting.cached_epochs().is_empty());

    // A revert is not the impossible class, so nothing is memoised: an operator can repair the
    // cause, and the next read succeeds without a restart.
    assert!(
        !err.is_contract_impossible(),
        "a revert is the contract refusing to answer, not an impossible answer"
    );
    assert!(
        reverting.poisoned_epochs().is_empty(),
        "a revert poisoned the epoch: a repaired contract would need a restart to be read"
    );
    let _ = reverting.committee(10);
    assert_eq!(
        reverting.reads().calls().snapshots_of(10),
        2,
        "the revert was memoised after all"
    );
}

#[test]
fn an_empty_committee_is_not_readable_and_never_permanent() {
    // The contract answers an uncommitted epoch with `Ok` and an empty array — "not yet", never
    // "never", so folding it into a permanent error would refuse an epoch about to exist.
    let at = hash(9);
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(
            Box::new(|_, _, _| Answer::Uncommitted),
            BTreeMap::from([(at, 325u64)]),
        ),
    );

    let err = store.committee(10).expect_err("uncommitted epoch");
    assert!(
        matches!(err, CommitteeError::NotReadable { epoch: 10, .. }),
        "got {err:?}"
    );
    assert!(err.is_transient());
    assert_eq!(
        store.reads().calls().quals_of(10),
        0,
        "an empty committee short-circuits before the second staticcall"
    );
}

#[test]
fn the_map_keeps_every_epoch_the_window_still_admits() {
    // Retention is the window floor, not a count: the map holds every epoch the window still
    // admits, so write-once covers every epoch the module will ever answer from.
    let (at, higher) = (hash(10), hash(15));
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor.clone(),
        FakeReads::new(
            four_members(),
            BTreeMap::from([(at, 325u64), (higher, 416u64)]),
        ),
    );

    // Anchor 325 is epoch 10 ⇒ window [2, 12]; anchor 416 is epoch 13 ⇒ [5, 15].
    for epoch in 2..=10u64 {
        store.committee(epoch).expect("inside the window");
    }
    assert_eq!(
        store.cached_epochs(),
        (2..=10).collect::<Vec<u64>>(),
        "every epoch of the window is still readable, so every one of them is still held"
    );
    for epoch in 2..=10u64 {
        store.committee(epoch).expect("cached");
        assert_eq!(
            store.reads().calls().snapshots_of(epoch),
            1,
            "epoch {epoch} is inside the window and was read once — it must cost nothing again"
        );
    }

    anchor.advance(416, higher);
    store.anchor_advanced();
    assert_eq!(
        store.cached_epochs(),
        (5..=10).collect::<Vec<u64>>(),
        "the retention rule is the window floor and moves with the anchor"
    );
    assert!(matches!(
        store.committee(4),
        Err(CommitteeError::OutOfWindow {
            epoch: 4,
            lo: 5,
            ..
        })
    ));
}

#[test]
fn an_epoch_at_the_window_floor_keeps_its_first_value() {
    // The floor epoch must keep its first value: retention by count would evict it while it is
    // still readable, and the next, different answer would land in the vacant slot unchecked.
    let at = hash(16);
    let plan: Plan = Box::new(|epoch, _, nth| {
        if epoch == 2 && nth > 0 {
            Answer::Committee(vec![2, 3, 4, 5])
        } else {
            Answer::Committee(vec![0, 1, 2, 3])
        }
    });
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(plan, BTreeMap::from([(at, 325u64)])),
    );

    let first = store
        .committee(2)
        .expect("the floor of the window is readable");
    for epoch in 3..=10u64 {
        store.committee(epoch).expect("inside the window");
    }

    let again = store.committee(2).expect("still inside the window");
    assert!(
        first.same_value(&again),
        "the first value of the floor epoch stands"
    );
    assert_eq!(
        store.reads().calls().snapshots_of(2),
        1,
        "the floor epoch was never re-read, so the second answer never had a vacant slot to land in"
    );
}

#[test]
fn the_wake_up_fires_on_every_advance_and_carries_the_anchor_height() {
    let anchor = FakeAnchor::at(0, hash(11));
    let store = new_store(
        anchor.clone(),
        FakeReads::new(four_members(), BTreeMap::new()),
    );

    let mut rx = store.anchor_advances();
    assert_eq!(*rx.borrow_and_update(), 0, "the anchor at birth");

    let target = geometry().commit_height(5);
    assert_eq!(target, geometry().start(3));
    anchor.advance(target, hash(12));
    store.anchor_advanced();

    assert!(rx.has_changed().unwrap(), "the wake-up fired");
    assert_eq!(
        *rx.borrow_and_update(),
        target,
        "the height the anchor holds"
    );

    // Every advance fires the event even when the height does not move: the other way a
    // consumer parks is an unexecuted anchor, and only a later advance can wake it.
    store.anchor_advanced();
    assert!(
        rx.has_changed().unwrap(),
        "an advance that leaves the height where it was is still a wake-up"
    );
    assert_eq!(*rx.borrow_and_update(), target);
}

#[test]
fn membership_and_the_facade_answer_from_the_same_record() {
    let at = hash(13);
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor.clone(),
        FakeReads::new(
            Box::new(|_, _, _| Answer::Committee(vec![0, 1, 2, 3])),
            BTreeMap::from([(at, 325u64)]),
        ),
    );

    let all = validators(6);
    let member = all[1].keys.peer_pubkey.clone();
    let outsider = all[5].keys.peer_pubkey.clone();
    assert!(store.is_member(10, &member).unwrap());
    assert!(!store.is_member(10, &outsider).unwrap());

    let record = store.committee(10).unwrap();
    let facade = CommitteeReadsFacade::new(store.clone() as Arc<dyn Committee>);

    assert_eq!(facade.read_at(), Some(at), "read_at IS the module's anchor");
    assert_eq!(
        facade.committee(10, B256::ZERO),
        Some(record.participants.clone())
    );
    assert_eq!(
        facade
            .committee_bls(10, B256::ZERO)
            .map(|c| c.bimap.into_iter().collect::<Vec<_>>()),
        Some(record.bls.bimap.clone().into_iter().collect::<Vec<_>>())
    );
    assert_eq!(
        facade.dkg_qual(10, B256::ZERO),
        Some((record.changed, true))
    );

    // The ignored `at` really is ignored: a hash this chain never sealed still answers.
    assert_eq!(
        facade.committee(10, hash(0xEE)),
        Some(record.participants.clone())
    );

    assert_eq!(facade.committee(99, B256::ZERO), None);
    assert!(facade.committee_bls(99, B256::ZERO).is_none());
    assert_eq!(facade.dkg_qual(99, B256::ZERO), None);
}

#[test]
fn a_zero_epoch_interval_has_no_geometry() {
    assert!(Geometry::new(100, 0).is_none());

    let g = Geometry::new(100, 32).unwrap();
    assert_eq!(g.epoch_of(99), 0, "pre-activation clamps");
    assert_eq!(g.epoch_of(100), 0);
    assert_eq!(g.epoch_of(132), 1);
    assert_eq!(g.start(2), 164);
    assert_eq!(g.last(2), 195);
    assert_eq!(g.commit_height(4), g.start(2));
}

#[test]
fn a_header_index_fault_at_the_anchor_is_permanent_not_a_park() {
    // The anchor documents a probe fault at a materialized height as a real fault, not a park:
    // folding it into `NotReadable` would strand a corruption behind a retry that cannot succeed.
    let at = hash(14);
    let anchor = FakeAnchor::at(325, at);
    anchor.fault.store(true, Ordering::Release);
    let store = new_store(
        anchor,
        FakeReads::new(four_members(), BTreeMap::from([(at, 325u64)])),
    );

    let err = store.committee(10).expect_err("probe fault");
    assert!(matches!(err, CommitteeError::Read(_)));
    assert!(!err.is_transient());
    assert_eq!(
        store.reads().calls(),
        Calls::default(),
        "no EVM after a bad anchor"
    );
}

#[test]
fn an_epoch_above_the_window_is_worth_a_retry_and_one_below_never_is() {
    // `OutOfWindow` is two answers behind one variant: above the window the anchor has not
    // reached the epoch yet, below it the weight ring has wrapped for good.
    let at = hash(17);
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(four_members(), BTreeMap::from([(at, 325u64)])),
    );

    let above = store.committee(13).expect_err("above the window");
    assert!(matches!(
        above,
        CommitteeError::OutOfWindow {
            epoch: 13,
            lo: 2,
            hi: 12
        }
    ));
    assert!(
        above.is_transient(),
        "the anchor only moves up, so epoch 13 becomes readable without anyone doing anything"
    );

    let below = store.committee(1).expect_err("below the window");
    assert!(matches!(
        below,
        CommitteeError::OutOfWindow { epoch: 1, .. }
    ));
    assert!(
        !below.is_transient(),
        "retrying an epoch under the floor is the eternal spin of a stuck slashing charge"
    );

    assert_eq!(
        store.reads().calls(),
        Calls::default(),
        "neither side of the window touches the EVM"
    );
}

#[test]
fn a_weight_vector_that_does_not_match_the_members_is_permanent() {
    // The module is generic over `EpochReads`, so the port's one-weight-per-member precondition
    // has to be a check rather than a hope.
    let at = hash(18);
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(
            Box::new(|_, _, _| Answer::CommitteeWeighted(vec![0, 1, 2, 3], vec![7, 7, 7])),
            BTreeMap::from([(at, 325u64)]),
        ),
    );

    let err = store
        .committee(10)
        .expect_err("three weights for four members");
    assert!(matches!(err, CommitteeError::Read(_)));
    assert!(
        !err.is_transient(),
        "a mismatched weight vector is the contract answering something impossible"
    );
    assert!(
        store.cached_epochs().is_empty(),
        "nothing half-built is kept"
    );
}

#[test]
fn a_failing_qual_leg_throws_the_snapshot_away_and_classifies_by_the_error() {
    // The one branch where the module drops a staticcall it already paid for: nothing is
    // cached, so the retry pays the snapshot again, and the split matches the first leg.
    let at = hash(19);
    let parked = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(four_members(), BTreeMap::from([(at, 325u64)])).with_qual(Box::new(
            |_, _| Err(ReadError::StateNotMaterialized { hash: B256::ZERO }),
        )),
    );

    let err = parked.committee(10).expect_err("the second leg failed");
    assert!(matches!(err, CommitteeError::Read(_)));
    assert!(err.is_transient());
    assert!(
        parked.cached_epochs().is_empty(),
        "half an answer is not an answer"
    );
    assert!(
        parked.reported_epochs().is_empty(),
        "a transient failure is the normal shape of a node catching up — it is not reported"
    );
    let _ = parked.committee(10);
    assert_eq!(
        parked.reads().calls().snapshots_of(10),
        2,
        "the retry really re-reads the snapshot it threw away"
    );

    let reverting = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(four_members(), BTreeMap::from([(at, 325u64)])).with_qual(Box::new(
            |_, _| Err(ReadError::CallReverted("execution reverted".into())),
        )),
    );
    let err = reverting.committee(10).expect_err("the qual call reverts");
    assert!(!err.is_transient(), "a revert is the contract speaking");
    assert!(reverting.cached_epochs().is_empty());
    assert_eq!(
        reverting.reported_epochs(),
        vec![10],
        "a permanent failure is logged once and then remembered, so it is logged once"
    );
}

#[test]
fn the_facade_folds_every_committee_error_to_none() {
    // The facade trait has no room for a reason and its consumers read `None` as "undecided,
    // ask again", so every variant has to arrive there.
    let at = hash(20);
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor.clone(),
        FakeReads::new(
            Box::new(|epoch, _, _| match epoch {
                11 => Answer::Uncommitted,
                12 => Answer::Failed("permanent"),
                _ => Answer::Committee(vec![0, 1, 2, 3]),
            }),
            BTreeMap::from([(at, 325u64)]),
        ),
    );

    assert!(matches!(
        store.committee(99),
        Err(CommitteeError::OutOfWindow { .. })
    ));
    assert!(matches!(
        store.committee(11),
        Err(CommitteeError::NotReadable { .. })
    ));
    assert!(matches!(store.committee(12), Err(CommitteeError::Read(_))));

    let facade = CommitteeReadsFacade::new(store.clone() as Arc<dyn Committee>);
    for epoch in [99u64, 11, 12] {
        assert_eq!(facade.committee(epoch, B256::ZERO), None);
        assert!(facade.committee_bls(epoch, B256::ZERO).is_none());
        assert_eq!(facade.dkg_qual(epoch, B256::ZERO), None);
    }
}

/// A reth provider carrying a persisted `finalized` tag, the one height a restarted node knows.
struct TaggedProvider {
    /// `finalized_block_number()`: `None` on a genuinely fresh execution layer.
    tag: Option<u64>,
    best: u64,
    /// The one hash `block_hash` answers, whatever height it is asked for.
    hash: B256,
}

impl reth_storage_api::BlockHashReader for TaggedProvider {
    fn block_hash(
        &self,
        _number: u64,
    ) -> reth_storage_api::errors::provider::ProviderResult<Option<B256>> {
        Ok(Some(self.hash))
    }
    fn canonical_hashes_range(
        &self,
        _start: u64,
        _end: u64,
    ) -> reth_storage_api::errors::provider::ProviderResult<Vec<B256>> {
        Ok(vec![])
    }
}

impl reth_storage_api::BlockNumReader for TaggedProvider {
    fn chain_info(
        &self,
    ) -> reth_storage_api::errors::provider::ProviderResult<reth_chainspec::ChainInfo> {
        Ok(reth_chainspec::ChainInfo::default())
    }
    fn best_block_number(&self) -> reth_storage_api::errors::provider::ProviderResult<u64> {
        Ok(self.best)
    }
    fn last_block_number(&self) -> reth_storage_api::errors::provider::ProviderResult<u64> {
        Ok(self.best)
    }
    fn block_number(
        &self,
        _hash: B256,
    ) -> reth_storage_api::errors::provider::ProviderResult<Option<u64>> {
        Ok(None)
    }
}

impl reth_storage_api::BlockIdReader for TaggedProvider {
    fn pending_block_num_hash(
        &self,
    ) -> reth_storage_api::errors::provider::ProviderResult<Option<alloy_eips::BlockNumHash>> {
        Ok(None)
    }
    fn safe_block_num_hash(
        &self,
    ) -> reth_storage_api::errors::provider::ProviderResult<Option<alloy_eips::BlockNumHash>> {
        Ok(None)
    }
    fn finalized_block_num_hash(
        &self,
    ) -> reth_storage_api::errors::provider::ProviderResult<Option<alloy_eips::BlockNumHash>> {
        Ok(self
            .tag
            .map(|number| alloy_eips::BlockNumHash::new(number, self.hash)))
    }
}

#[test]
fn a_persisted_finalized_tag_anchors_the_window_before_the_cursor_is_seeded() {
    // Reth carries a finalized tag from an earlier run while the process-local `FinalizedCursor`
    // is still zero; anchoring on the cursor alone would put the window at `[0, 2]`.
    const TAG: u64 = 400;
    let tag_epoch = geometry().epoch_of(TAG);
    let at = hash(19);
    let cursor = crate::FinalizedCursor::default();
    let anchor = Arc::new(RethAnchor::new(
        cursor.clone(),
        TaggedProvider {
            tag: Some(TAG),
            best: TAG,
            hash: at,
        },
    ));
    assert_eq!(
        anchor.height(),
        TAG,
        "an unseeded cursor must not hide a height this node has finalized"
    );

    let store = CommitteeStore::new(
        FakeReads::new(four_members(), BTreeMap::from([(at, TAG)])),
        anchor.clone(),
        frozen_geometry_rx(),
        test_verifier(),
    );
    match store.committee(9_999).expect_err("far above any window") {
        CommitteeError::OutOfWindow { lo, hi, .. } => assert_eq!(
            (lo, hi),
            (tag_epoch - SCHEME_RETENTION_EPOCHS as u64, tag_epoch + 2),
            "the window is anchored on the tag's epoch, not on epoch 0"
        ),
        other => panic!("expected OutOfWindow, got {other:?}"),
    }
    store
        .committee(tag_epoch)
        .expect("the epoch the tag sits in is readable at the tag's own state");

    // The tag is a floor, not a replacement: once the cursor passes it, the cursor wins again.
    cursor.advance(TAG + 3 * INTERVAL);
    assert_eq!(anchor.height(), TAG + 3 * INTERVAL);
}

#[test]
fn an_execution_layer_with_no_finalized_tag_leaves_the_cursor_alone() {
    // A genuinely fresh node: no tag reads as 0, so the anchor is exactly the cursor.
    let at = hash(21);
    let cursor = crate::FinalizedCursor::default();
    let anchor = RethAnchor::new(
        cursor.clone(),
        TaggedProvider {
            tag: None,
            best: 0,
            hash: at,
        },
    );
    assert_eq!(anchor.height(), 0);
    cursor.advance(64);
    assert_eq!(anchor.height(), 64);
}

/// The narrowest [`SeedOracle`]: the map only asks whether an oracle is there, never for an
/// answer.
#[derive(Debug)]
struct StubOracle;

impl fluentbase_bls::oracle::SeedOracle for StubOracle {
    fn sign_partial(
        &self,
        _round: commonware_consensus::types::Round,
    ) -> Option<fluentbase_bls::BlsSignature> {
        None
    }
    fn verify_partial(
        &self,
        _round: commonware_consensus::types::Round,
        _index: commonware_utils::Participant,
        _v: &fluentbase_bls::BlsSignature,
    ) -> bool {
        false
    }
    fn recover(
        &self,
        _partials: &[(commonware_utils::Participant, fluentbase_bls::BlsSignature)],
        _threshold: u32,
    ) -> Option<fluentbase_bls::BlsSignature> {
        None
    }
    fn verify_seed(
        &self,
        _round: commonware_consensus::types::Round,
        _seed: &fluentbase_bls::BlsSignature,
    ) -> fluentbase_bls::oracle::SeedCheck {
        fluentbase_bls::oracle::SeedCheck::NoKey
    }
}

/// `n` validators in the contract's order, with the keypairs that produced them: a signer
/// scheme needs a keypair the committee actually contains.
fn validators_with_keys(n: usize) -> (Vec<ValidatorWithKeys>, Vec<ValidatorBlsKeypair>) {
    let mut rng = StdRng::seed_from_u64(0x5164);
    let mut pairs: Vec<(ValidatorWithKeys, ValidatorBlsKeypair)> = (0..n)
        .map(|i| {
            let peer = Ed25519PrivateKey::random(&mut rng).public_key();
            let bls = ValidatorBlsKeypair::generate(&mut rng);
            (
                ValidatorWithKeys {
                    address: alloy_primitives::Address::with_last_byte(i as u8 + 1),
                    keys: ConsensusKeys {
                        bls_pubkey: BlsPubkey::decode(bls.public_bytes().as_slice()).unwrap(),
                        peer_pubkey: peer,
                        activation_epoch: 0,
                    },
                    tombstoned: false,
                },
                bls,
            )
        })
        .collect();
    pairs.sort_by(|a, b| a.0.keys.peer_pubkey.cmp(&b.0.keys.peer_pubkey));
    pairs.into_iter().unzip()
}

/// A store over the given committee whose producer builds a verify-only scheme with or without
/// an oracle, so a test can drive each refusal from the outside.
fn store_over(
    members: Vec<ValidatorWithKeys>,
    beacon_active: bool,
) -> Arc<CommitteeStore<FakeReads>> {
    let at = hash(31);
    let anchor = FakeAnchor::at(325, at);
    let mut reads = FakeReads::new(
        Box::new(|_, _, _| Answer::Committee(vec![0, 1, 2, 3])),
        BTreeMap::from([(at, 325u64)]),
    );
    reads.validators = members;
    Arc::new(CommitteeStore::new(
        reads,
        anchor,
        frozen_geometry_rx(),
        Arc::new(move |record: &CommitteeRecord| {
            Some(fluentbase_bls::scheme::build_verifier(
                &fluentbase_bls::fluent_namespace(CHAIN_ID),
                record.bls.bimap.clone(),
                record.epoch,
                beacon_active
                    .then(|| Arc::new(StubOracle) as Arc<dyn fluentbase_bls::oracle::SeedOracle>),
            ))
        }),
    ))
}

/// A signer is never replaced by a verifier, while the verifier→signer upgrade — the whole
/// reason `upgrade_scheme` exists — still lands.
#[test]
fn a_scheme_upgrade_refuses_a_signer_to_verifier_downgrade_but_accepts_the_upgrade() {
    use commonware_cryptography::certificate::Scheme as _;
    const EPOCH: u64 = 10;
    let (members, keypairs) = validators_with_keys(6);
    let store = store_over(members, false);
    let ns = fluentbase_bls::fluent_namespace(CHAIN_ID);

    let record = store.committee(EPOCH).expect("in the window");
    let bimap = record.bls.bimap.clone();
    // The plan takes member slots 0..3, so slot 0's keypair is in this committee.
    let signer =
        fluentbase_bls::scheme::build_signer(&ns, bimap.clone(), &keypairs[0], EPOCH, None)
            .expect("keypair is a committee member");
    let verifier = || fluentbase_bls::scheme::build_verifier(&ns, bimap.clone(), EPOCH, None);
    let is_signer = || store.scheme(EPOCH).expect("installed").me().is_some();

    assert!(!is_signer(), "the module's own entry is verify-only");
    assert!(store.upgrade_scheme(EPOCH, signer));
    assert!(is_signer());

    assert!(!store.upgrade_scheme(EPOCH, verifier()));
    assert!(is_signer());

    let (other_members, _) = validators_with_keys(5);
    let other = EpochCommittee::from_pairs(
        EPOCH,
        other_members
            .iter()
            .take(4)
            .map(|v| (v.keys.peer_pubkey.clone(), v.keys.bls_pubkey)),
    )
    .expect("unique keys")
    .bimap;
    assert!(!store.upgrade_scheme(
        EPOCH,
        fluentbase_bls::scheme::build_verifier(&ns, other, EPOCH, None)
    ));
    assert!(is_signer());
}

/// Upgrades are monotone in verification strength, and the oracle is the only strength neither
/// `participants()` nor `me()` can see: over one committee, an oracle-less scheme admits a
/// certificate whose seed slot was cleared where the beacon-active one refuses it.
#[test]
fn a_scheme_upgrade_refuses_a_replacement_that_drops_the_beacon_oracle() {
    const EPOCH: u64 = 10;
    let (members, _) = validators_with_keys(6);
    let ns = fluentbase_bls::fluent_namespace(CHAIN_ID);

    let store = store_over(members.clone(), true);
    let record = store.committee(EPOCH).expect("in the window");
    let bimap = record.bls.bimap.clone();
    assert!(store.scheme(EPOCH).expect("installed").is_beacon_active());

    assert!(!store.upgrade_scheme(
        EPOCH,
        fluentbase_bls::scheme::build_verifier(&ns, bimap.clone(), EPOCH, None)
    ));
    assert!(
        store.scheme(EPOCH).expect("installed").is_beacon_active(),
        "the oracle-less replacement must be refused — it admits a cleared seed \
         slot where the entry it would replace refuses one"
    );

    // The other direction still lands: an oracle-less entry is replaced by the beacon-active one.
    let cold = store_over(members, false);
    let cold_record = cold.committee(EPOCH).expect("in the window");
    assert!(!cold.scheme(EPOCH).expect("installed").is_beacon_active());
    assert!(cold.upgrade_scheme(
        EPOCH,
        fluentbase_bls::scheme::build_verifier(
            &ns,
            cold_record.bls.bimap.clone(),
            EPOCH,
            Some(Arc::new(StubOracle)),
        )
    ));
    assert!(cold.scheme(EPOCH).expect("installed").is_beacon_active());
}

/// `upgrade_scheme` cannot create an entry, which is what makes "a scheme exists exactly when
/// this node read the committee it verifies under" structural rather than a convention.
#[test]
fn a_scheme_upgrade_refuses_an_epoch_with_no_committee_record() {
    const READ: u64 = 10;
    const UNREAD: u64 = 11;
    let (members, _) = validators_with_keys(6);
    let store = store_over(members, false);
    let ns = fluentbase_bls::fluent_namespace(CHAIN_ID);

    // One epoch read, so the refusal below cannot be confused with "this store answers nothing".
    let bimap = store
        .committee(READ)
        .expect("in the window")
        .bls
        .bimap
        .clone();
    assert_eq!(store.cached_epochs(), vec![READ]);

    // `UNREAD` is inside the window and would be readable on demand, so the refusal is about
    // the absence of a record, not about the epoch being out of reach.
    assert!(
        !store.upgrade_scheme(
            UNREAD,
            fluentbase_bls::scheme::build_verifier(&ns, bimap, UNREAD, None)
        ),
        "a scheme for an epoch whose committee this node never read must be refused"
    );
    assert_eq!(
        store.cached_epochs(),
        vec![READ],
        "the refused upgrade must not have created an entry"
    );
    assert!(
        store.scheme(UNREAD).is_some(),
        "and the epoch is still readable the ONE way it can be — by reading its committee"
    );
    assert_eq!(
        store.cached_epochs(),
        vec![READ, UNREAD],
        "which is the only thing that ever puts an epoch in this map"
    );
}
