//! Module tests.
//!
//! The fakes here are deliberately NOT `testbed::fakes::FakeStaking`. That one
//! answers a committee as a pure function of the EPOCH — the hash only decides
//! "is it committed yet" — so "two nodes at different heights read the same
//! committee" would be a tautology over it. The reader below BRANCHES ON THE
//! HASH, can withhold weights, can answer empty, can fail transiently or
//! permanently, and counts every staticcall by `(epoch, at)`. Those are the
//! properties of the CALLER this module is asserting, so the fake has to be able
//! to break each of them.

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

/// `n` validators in CONTRACT ORDER (ascending on the raw peer pubkey), which
/// is the order `commitEpochCommittee` sorts its array in and therefore the
/// only order a real snapshot can arrive in.
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

// ---------------------------------------------------------------- the anchor

/// A movable ordering-finalized cursor plus a movable executed-state probe.
/// The two are independent on purpose: "the height advanced" and "the state at
/// that height is materialized" are exactly the two things the production
/// anchor keeps apart.
struct FakeAnchor {
    height: AtomicU64,
    /// height → executed hash. A height ABSENT from here is the
    /// `executed_state_hash` park (`Ok(None)`).
    executed: Mutex<BTreeMap<u64, B256>>,
    /// Force the probe to answer a real fault instead.
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

    /// An anchor whose height is set but whose state is NOT materialized.
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

// ---------------------------------------------------------------- the reader

/// What the contract answers for one `(epoch, at)`.
#[derive(Clone)]
enum Answer {
    /// A committed committee of these member slots, with frozen weights.
    Committee(Vec<usize>),
    /// A committed committee whose frozen weights are handed over VERBATIM —
    /// the only way to hand the module a weight vector whose length does not
    /// match the member array.
    CommitteeWeighted(Vec<usize>, Vec<u128>),
    /// Committed, but the contract's weight ring has wrapped past this epoch.
    WeightsWrapped(Vec<usize>),
    /// Not committed in the state at `at`.
    Uncommitted,
    /// The read itself fails.
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

/// Every staticcall the module made, as `(epoch, hash)` in call order.
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

/// What `getDkgQual(epoch)` answers at `at`. A leg of its own, and a FALLIBLE
/// one: by the time the module asks for the bit it has already paid for the
/// snapshot call, and it branches on the hash like the other leg, so a test
/// that asserts two anchors agree on the bit is asserting something the fake
/// could have contradicted.
type QualPlan = Box<dyn Fn(u64, B256) -> Result<bool, ReadError> + Send + Sync>;

/// The everyday bit: the contract writes it in the same call that commits the
/// committee, so on any in-window anchor it is a function of the epoch alone.
fn qual_by_epoch() -> QualPlan {
    Box::new(|epoch, _| Ok(epoch % 2 == 1))
}

struct FakeReads {
    validators: Vec<ValidatorWithKeys>,
    /// `(epoch, at, how many snapshot reads of this epoch came before)`.
    plan: Plan,
    /// hash → block number, so the snapshot can report the height it was taken
    /// at the way the real reader does (from the header it already read).
    heights: BTreeMap<B256, u64>,
    /// The second staticcall, answered independently of the first.
    qual: QualPlan,
    calls: Mutex<Calls>,
    /// Held by the two-thread write-once test so both readers miss the map.
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

/// The everyday plan: every epoch is a committed four-member committee.
fn four_members() -> Plan {
    Box::new(|_, _, _| Answer::Committee(vec![0, 1, 2, 3]))
}

/// A watch already carrying the frozen pair — the state the store spends its
/// whole life in once the plane has frozen the geometry. The sender is leaked
/// into the receiver by `watch::channel` semantics only while it lives, so it
/// is kept alive by the returned receiver's own channel (a `Sender` dropped
/// here would make `borrow()` still read the last value, which is all the store
/// ever does).
fn frozen_geometry_rx() -> crate::committee::GeometryRx {
    tokio::sync::watch::Sender::new(Some((0, INTERVAL))).subscribe()
}

fn new_store(anchor: Arc<FakeAnchor>, reads: FakeReads) -> Arc<CommitteeStore<FakeReads>> {
    Arc::new(CommitteeStore::new(reads, anchor, frozen_geometry_rx()))
}

// --------------------------------------------------------- 0. no geometry yet

#[test]
fn without_a_frozen_geometry_every_epoch_is_not_readable_without_a_single_read() {
    // The startup state of a real node: the store is built beside the anchor,
    // BEFORE the beacon plane's EpochTransition has reached a readable,
    // DPoS-scheduled block and frozen `(activation, interval)`. With no epoch
    // arithmetic there is no window and no commit height, so there is nothing
    // to read and nothing to ask the EVM.
    let (tx, rx) = tokio::sync::watch::channel(None);
    let anchor = FakeAnchor::at(400, hash(7));
    let store = Arc::new(CommitteeStore::new(
        FakeReads::new(four_members(), BTreeMap::from([(hash(7), 400u64)])),
        anchor,
        rx,
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
    // The wake-up is seeded at 0 rather than at a bogus epoch, and an advance
    // taken before the freeze publishes nothing (there is nothing to publish).
    let mut readable = store.subscribe();
    assert_eq!(*readable.borrow_and_update(), 0);
    store.anchor_advanced();
    assert!(
        !readable.has_changed().expect("sender alive"),
        "no geometry, no readable epoch"
    );

    // The freeze lands and the SAME store starts answering — no rebuild, no
    // second cursor.
    tx.send_replace(Some((0, INTERVAL)));
    store
        .committee(5)
        .expect("readable once the geometry is frozen");
    store.anchor_advanced();
    assert_eq!(
        *readable.borrow_and_update(),
        geometry().epoch_of(400) + 2,
        "the first advance after the freeze publishes the real ceiling"
    );
}

// ------------------------------------------------------------------- 1. gate

#[test]
fn an_epoch_below_its_commit_height_is_not_readable_without_a_single_read() {
    // `commit_height(E) = start(E−2)` EXCEPT for the first three epochs, which
    // `start` cannot express: genesis is not executed by the ahead-commit
    // drain, the bootstrap commits epoch 0 alone, and epochs 1 and 2 are first
    // committed by the first EXECUTED block — height 1, not 0. So an anchor
    // still at height 0 can read epoch 0 and must refuse 1 and 2.
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

// ------------------------------------------------------- 2. unexecuted anchor

#[test]
fn an_anchor_whose_state_is_not_materialized_is_not_readable_without_a_single_read() {
    // The backfill case (R-123): reth wrote the header but not the state. The
    // probe answers `Ok(None)` and the module parks WITHOUT a staticcall —
    // which is the whole reason the anchor is a state probe and not
    // `block_hash`.
    let anchor = FakeAnchor::unexecuted(320);
    let store = new_store(anchor, FakeReads::new(four_members(), BTreeMap::new()));

    assert!(matches!(
        store.committee(10),
        Err(CommitteeError::NotReadable { epoch: 10, .. })
    ));
    assert_eq!(store.reads().calls(), Calls::default());
}

// ------------------------------------------------------------- 3. the window

#[test]
fn an_epoch_outside_the_window_is_refused_without_a_single_read_and_the_borders_are_inside() {
    // anchor in epoch 10 ⇒ window [10 − SCHEME_RETENTION_EPOCHS, 10 + 2] = [2, 12].
    let at = hash(3);
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor,
        FakeReads::new(four_members(), BTreeMap::from([(at, 325)])),
    );

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

    // Both borders are INSIDE, so the refusals above are the window and not an
    // off-by-one that would have refused a legitimate epoch too.
    for border in [2u64, 12] {
        assert!(
            store.committee(border).is_ok(),
            "epoch {border} is on the window border and must be readable"
        );
    }
}

// ------------------------------------------------ 4. one pair of staticcalls

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

// --------------------------------------------- 5. two anchors, one record

#[test]
fn two_anchors_on_different_branches_produce_the_same_record() {
    // The fake genuinely branches on the hash — epoch 7 answers a DIFFERENT
    // committee per branch — so the equality asserted for epoch 5 is a property
    // of this module and not of a fake that could not have disagreed.
    let (a, b) = (hash(0xAA), hash(0xBB));
    let heights = BTreeMap::from([(a, 200u64), (b, 260u64)]);

    // epoch(200) = 6 ⇒ window [0, 8]; epoch(260) = 8 ⇒ window [0, 10].
    // Epochs 5 and 7 are inside both.
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

/// The qual leg of the same contract: epoch 7 answers a different BIT per
/// branch, every other epoch answers the same one on both.
fn branching_qual(a: B256) -> QualPlan {
    Box::new(move |epoch, at| match epoch {
        7 => Ok(at == a),
        _ => Ok(epoch % 2 == 1),
    })
}

/// A contract whose epoch-7 answer DEPENDS ON THE BRANCH and whose epoch-5
/// answer does not — rebuilt per store so each gets its own boxed closure.
fn branching(a: B256, b: B256) -> Plan {
    Box::new(move |epoch, at, _| match (epoch, at) {
        (7, x) if x == a => Answer::Committee(vec![0, 1, 2, 3]),
        (7, x) if x == b => Answer::Committee(vec![2, 3, 4, 5]),
        _ => Answer::Committee(vec![0, 1, 2, 3]),
    })
}

// -------------------------------------------------------------- 6. write-once

#[test]
fn a_second_answer_for_one_epoch_is_refused_and_the_first_record_stands() {
    // The occupied arm is reached the ONLY way production can reach it: two
    // consumers miss the map for one epoch and both issue their own pair of
    // staticcalls, because the map lock is deliberately NOT held across a
    // blocking state read. The barrier makes that race deterministic; the plan
    // hands the two readers two DIFFERENT committees, i.e. a contract fork.
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

    let stored = store
        .committee(10)
        .expect("the installed record still answers");
    assert!(
        stored.same_value(kept[0]),
        "the FIRST record stands; the second never overwrites it"
    );
    assert_eq!(
        store.reads().calls().snapshots_of(10),
        2,
        "no third read: the refusal did not evict the record"
    );
}

// ---------------------------------------------------------- 7. absent weights

#[test]
fn absent_weights_inside_the_window_are_permanent_and_cache_nothing() {
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

    // Nothing was cached: the next call goes to the contract again.
    let _ = store.committee(10);
    assert_eq!(store.reads().calls().snapshots_of(10), 2);
}

// --------------------------------------------------------- 8. transient error

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

    // The other half of the same predicate: a revert is the contract SPEAKING,
    // and it will say the same thing on every retry.
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
}

// -------------------------------------------------------- 9. empty committee

#[test]
fn an_empty_committee_is_not_readable_and_never_permanent() {
    // The contract answers an uncommitted epoch with `Ok` and an empty array.
    // It only ever skips a commit together with halting the chain (E4-25), so
    // this is "not yet", never "never" — folding it into a permanent error
    // would refuse an epoch that is about to exist.
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

// ------------------------------------------------------------- 10. retention

#[test]
fn the_map_keeps_every_epoch_the_window_still_admits() {
    // Retention is the window FLOOR, not a count. The window is 11 epochs deep
    // and the map holds all of them, so the write-once value — and with it the
    // contract-fork detector — covers every epoch the module will ever answer
    // from. A count of SCHEME_RETENTION_EPOCHS would have evicted the bottom of
    // its own window.
    let (at, higher) = (hash(10), hash(15));
    let anchor = FakeAnchor::at(325, at);
    let store = new_store(
        anchor.clone(),
        FakeReads::new(
            four_members(),
            BTreeMap::from([(at, 325u64), (higher, 416u64)]),
        ),
    );

    // Window at epoch 10 is [2, 12]; read 2..=10, i.e. more epochs than
    // SCHEME_RETENTION_EPOCHS.
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

    // Raise the anchor into epoch 13. The floor becomes 5, epochs 2..=4 leave
    // the window for good, and the map drops exactly them — nothing that is
    // still answerable, nothing that is not.
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

// ------------------------------------------- 10b. write-once at the floor

#[test]
fn an_epoch_at_the_window_floor_keeps_its_first_value() {
    // What retention-by-count actually cost: the OLDEST epoch of the window was
    // evicted while still readable, and a second, DIFFERENT answer for it found
    // a VACANT slot and was installed silently — the exact contract fork
    // write-once exists to refuse. With the floor rule the epoch is never
    // re-read at all.
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

// ------------------------------------------------------------- 11. subscribe

#[test]
fn the_wake_up_carries_the_highest_readable_epoch_as_the_anchor_grows() {
    let anchor = FakeAnchor::at(0, hash(11));
    let store = new_store(
        anchor.clone(),
        FakeReads::new(four_members(), BTreeMap::new()),
    );

    let mut rx = store.subscribe();
    assert_eq!(*rx.borrow_and_update(), 2, "epoch(0) + 2");

    // Raise the anchor to the height that first commits epoch 5.
    let target = geometry().commit_height(5);
    assert_eq!(target, geometry().start(3));
    anchor.advance(target, hash(12));
    store.anchor_advanced();

    assert!(rx.has_changed().unwrap(), "the wake-up fired");
    assert_eq!(*rx.borrow_and_update(), 5, "epoch(anchor) + 2");

    // Monotone: an anchor that did not move publishes nothing.
    store.anchor_advanced();
    assert!(!rx.has_changed().unwrap());
}

// ------------------------------------------------- 12. membership + the facade

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

    // The ignored `at` really is ignored: a hash this chain never sealed still
    // answers the record, because the record does not depend on it.
    assert_eq!(
        facade.committee(10, hash(0xEE)),
        Some(record.participants.clone())
    );

    // Every CommitteeError folds to `None`, including the two that never touch
    // the EVM.
    assert_eq!(facade.committee(99, B256::ZERO), None);
    assert!(facade.committee_bls(99, B256::ZERO).is_none());
    assert_eq!(facade.dkg_qual(99, B256::ZERO), None);
}

// -------------------------------------------------------------- 13. geometry

#[test]
fn a_zero_epoch_interval_has_no_geometry() {
    // What makes every method on `Geometry` total: the undefined division is
    // refused at CONSTRUCTION, so no read path has to carry a "what if the
    // interval is zero" arm.
    assert!(Geometry::new(100, 0).is_none());

    let g = Geometry::new(100, 32).unwrap();
    assert_eq!(g.epoch_of(99), 0, "pre-activation clamps");
    assert_eq!(g.epoch_of(100), 0);
    assert_eq!(g.epoch_of(132), 1);
    assert_eq!(g.start(2), 164);
    assert_eq!(g.last(2), 195);
    assert_eq!(g.commit_height(4), g.start(2));
}

// ----------------------------------------------------- 14. anchor probe fault

#[test]
fn a_header_index_fault_at_the_anchor_is_permanent_not_a_park() {
    // `executed_state_hash` documents a `block_hash` miss at a MATERIALIZED
    // height as a real fault rather than a park. Folding it into `NotReadable`
    // would strand a corruption behind a retry that can never succeed.
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

// -------------------------------------------------- 15. the window has a side

#[test]
fn an_epoch_above_the_window_is_worth_a_retry_and_one_below_never_is() {
    // `OutOfWindow` is two different answers behind one variant. ABOVE the
    // window the anchor simply has not got there yet and moves up on its own;
    // BELOW it the contract's weight ring has wrapped and no anchor will ever
    // bring the epoch back. A consumer routing on the predicate (§5.4 says the
    // slasher does) would otherwise refuse forever an epoch that becomes
    // readable in a block or two.
    let at = hash(17);
    let store = new_store(
        FakeAnchor::at(325, at),
        FakeReads::new(four_members(), BTreeMap::from([(at, 325u64)])),
    );

    // Window at epoch 10 is [2, 12].
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

// ------------------------------------------------- 16. one weight per member

#[test]
fn a_weight_vector_that_does_not_match_the_members_is_permanent() {
    // `CommitteeRecord::weights` is documented as one weight per member in the
    // same order, and the leader elector indexes it positionally. The reader
    // forces the length for the production reader, but the module is generic
    // over `EpochReads` on purpose, so the unwritten precondition of the port
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

// ------------------------------------------------------- 17. the second leg

#[test]
fn a_failing_qual_leg_throws_the_snapshot_away_and_classifies_by_the_error() {
    // The one branch where the module has already paid for a staticcall and
    // drops it. Nothing is cached, so the retry costs the snapshot again, and
    // the transient/permanent split is the SAME split as on the first leg.
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

// ------------------------------------------- 18. the facade folds every error

#[test]
fn the_facade_folds_every_committee_error_to_none() {
    // The trait has no room for a reason and all three of its consumers read
    // `None` as "undecided, ask again", so every variant has to arrive there —
    // not just the one a single test happened to produce.
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

    // The three variants really are three different variants.
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

// ------------------------------------------------- 19. the production anchor

/// A reth provider carrying a persisted `finalized` tag — the one number a
/// restarted node knows before its consensus layer has re-derived anything.
struct TaggedProvider {
    /// `finalized_block_number()`: reth's own tag, `None` on a genuinely fresh
    /// execution layer.
    tag: Option<u64>,
    /// `best_block_number()`: the materialized head.
    best: u64,
    /// What `block_hash` resolves to at any height at or below `best`.
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
    // The startup state this exists for: reth carries a finalized tag from an
    // earlier run (or from the cold-start jump that just landed), while the
    // `FinalizedCursor` — a PROCESS quantity, seeded inside
    // `OuterBuilder::build` — is still zero. Anchoring on the cursor alone puts
    // the window at `[0, 2]` and blinds the node to every epoch of the chain it
    // is demonstrably following.
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
    );
    // The WINDOW is the tag's, which is the observable that matters: every
    // consumer's refusal is a function of it.
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

    // And the tag is a FLOOR, not a replacement: once the executor seeds and
    // raises the cursor past it, the cursor is the anchor again.
    cursor.advance(TAG + 3 * INTERVAL);
    assert_eq!(anchor.height(), TAG + 3 * INTERVAL);
}

#[test]
fn an_execution_layer_with_no_finalized_tag_leaves_the_cursor_alone() {
    // The other startup: a genuinely fresh node, no tag at all. `None` reads as
    // 0 and the anchor is exactly the cursor — the floor adds nothing and, in
    // particular, does not invent a height.
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
