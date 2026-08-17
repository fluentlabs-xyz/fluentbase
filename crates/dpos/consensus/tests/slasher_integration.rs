//! Integration: slasher pipeline under the commonware deterministic runtime.
//!
//! Two test groups:
//!
//! 1. **Reporter multiplex routing** (pre-existing): proves the
//!    `Reporters::from((marshal, slasher))` multiplex fans out
//!    `ConflictingNotarize` events to BOTH arms and that
//!    `extract_from_conflicting_notarize` re-decodes the event into a
//!    SlashCallArgs containing the offender's signer index.
//!
//! 2. **Full actor pipeline**: the producer/consumer split using a
//!    real `commonware_storage::queue::shared` WAL, a recording
//!    [`SlasherTxSink`] stub, and a configurable [`StakingStateRead`]
//!    stub.
//!
//! A charge is block-eligible only inside its own epoch, so the transaction
//! route these cases exercise is the **epoch-boundary fallback**: a charge is
//! held for a proposer while its epoch runs and only reaches the sink once the
//! epoch turns. Every case here therefore closes the epoch after driving its
//! event — see [`report_and_close_epoch`].

use alloy_primitives::{Address, B256};
use commonware_codec::DecodeExt;
use commonware_consensus::{
    simplex::types::{
        Activity, Attributable, ConflictingFinalize, ConflictingNotarize, Finalize, Notarize,
        Nullify, NullifyFinalize, Proposal, Vote,
    },
    types::{Epoch, Round, View},
    Reporter, Reporters,
};
use commonware_cryptography::{
    ed25519::PrivateKey as Ed25519PrivateKey, sha256::Digest as Sha256Digest, Signer,
};
use commonware_math::algebra::Random;
use commonware_runtime::Runner as _;
use commonware_utils::{ordered::BiMap, TryCollect};
use fluentbase_bls::{
    fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey, PeerPubkey,
    Scheme as BlsScheme,
};
use fluentbase_consensus::slasher::{
    self,
    actor::{SlasherTxSink, StaleEpochFallback, SubmitOutcome},
    evidence::{extract_from_conflicting_notarize, SlashKind},
};
use fluentbase_staking_reader::{
    error::ReadError,
    reader::{ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys},
    StakingStateRead,
};
use rand_08::rngs::StdRng;
use rand_core::SeedableRng;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex as TokioMutex};

const C_MAIN: u64 = 20_994;
const COMMITTEE_N: usize = 4;
const OFFENDER: usize = 0;
const EPOCH: u64 = 7;
const VIEW: u64 = 42;

fn committee(seed: u64) -> (Vec<ValidatorBlsKeypair>, BiMap<PeerPubkey, BlsPubkey>) {
    let mut rng = StdRng::seed_from_u64(seed);
    let peer_sks: Vec<_> = (0..COMMITTEE_N)
        .map(|_| Ed25519PrivateKey::random(&mut rng))
        .collect();
    let bls_kps: Vec<_> = (0..COMMITTEE_N)
        .map(|_| ValidatorBlsKeypair::generate(&mut rng))
        .collect();
    let bimap: BiMap<_, _> = peer_sks
        .iter()
        .zip(bls_kps.iter())
        .map(|(p, b)| {
            (
                p.public_key(),
                BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
            )
        })
        .try_collect()
        .unwrap();
    (bls_kps, bimap)
}

fn digest(tag: u8) -> Sha256Digest {
    let mut d = [0u8; 32];
    d[0] = tag;
    d[31] = tag;
    Sha256Digest::from(d)
}

fn build_conflicting_notarize() -> (
    ConflictingNotarize<BlsScheme, Sha256Digest>,
    BiMap<PeerPubkey, BlsPubkey>,
) {
    let (kps, bimap) = committee(1);
    let s = build_signer(
        &fluent_namespace(C_MAIN),
        bimap.clone(),
        &kps[OFFENDER],
        None,
    )
    .expect("offender must be in committee");
    let round = Round::new(Epoch::new(EPOCH), View::new(VIEW));
    let p1: Proposal<Sha256Digest> = Proposal::new(round, View::new(VIEW - 1), digest(0xaa));
    let p2: Proposal<Sha256Digest> = Proposal::new(round, View::new(VIEW - 1), digest(0xbb));
    let n1 = Notarize::sign(&s, p1).expect("sign n1");
    let n2 = Notarize::sign(&s, p2).expect("sign n2");
    (ConflictingNotarize::new(n1, n2), bimap)
}

fn build_consensus_digest_conflicting_notarize() -> (
    ConflictingNotarize<BlsScheme, fluentbase_consensus::Digest>,
    Vec<ValidatorBlsKeypair>,
    BiMap<PeerPubkey, BlsPubkey>,
) {
    let (kps, bimap) = committee(1);
    let s = build_signer(
        &fluent_namespace(C_MAIN),
        bimap.clone(),
        &kps[OFFENDER],
        None,
    )
    .expect("offender must be in committee");
    let round = Round::new(Epoch::new(EPOCH), View::new(VIEW));
    let d_a = fluentbase_consensus::Digest(B256::from([0xaa; 32]));
    let d_b = fluentbase_consensus::Digest(B256::from([0xbb; 32]));
    let p1: Proposal<fluentbase_consensus::Digest> = Proposal::new(round, View::new(VIEW - 1), d_a);
    let p2: Proposal<fluentbase_consensus::Digest> = Proposal::new(round, View::new(VIEW - 1), d_b);
    let n1 = Notarize::sign(&s, p1).expect("sign n1");
    let n2 = Notarize::sign(&s, p2).expect("sign n2");
    (ConflictingNotarize::new(n1, n2), kps, bimap)
}

/// One signed notarize by OFFENDER. On its own it is not slashable — two of
/// them for one round with different proposals are, and a single one is also
/// the cheapest activity that tells the slasher which epoch consensus is in.
fn offender_notarize(
    epoch: u64,
    view: u64,
    tag: u8,
) -> Activity<BlsScheme, fluentbase_consensus::Digest> {
    let (kps, bimap) = committee(1);
    let s = build_signer(&fluent_namespace(C_MAIN), bimap, &kps[OFFENDER], None)
        .expect("offender in committee");
    let round = Round::new(Epoch::new(epoch), View::new(view));
    let proposal = Proposal::new(
        round,
        View::new(view - 1),
        fluentbase_consensus::Digest(B256::from([tag; 32])),
    );
    Activity::Notarize(Notarize::sign(&s, proposal).expect("offender signs"))
}

fn snapshot_from_bimap(bimap: &BiMap<PeerPubkey, BlsPubkey>) -> ValidatorSetSnapshot {
    let validators = bimap
        .iter_pairs()
        .enumerate()
        .map(|(i, (peer, bls))| {
            let mut addr_bytes = [0u8; 20];
            addr_bytes[19] = (i + 1) as u8;
            ValidatorWithKeys {
                address: Address::from(addr_bytes),
                keys: ConsensusKeys {
                    bls_pubkey: *bls,
                    peer_pubkey: peer.clone(),
                    activation_epoch: 0,
                },
                stake: 1,
                tombstoned: false,
            }
        })
        .collect();
    ValidatorSetSnapshot {
        block_hash: B256::ZERO,
        block_number: 0,
        epoch: EPOCH,
        validators,
    }
}

#[derive(Clone)]
struct StubReader {
    snapshot: ValidatorSetSnapshot,
    empty: bool,
}

impl StakingStateRead for StubReader {
    fn epoch_committee_snapshot(
        &self,
        _epoch: u64,
        _at: B256,
    ) -> Result<ValidatorSetSnapshot, ReadError> {
        if self.empty {
            // Simulate the contract's prune cursor having advanced past
            // this epoch by returning an empty validator set.
            Ok(ValidatorSetSnapshot {
                block_hash: B256::ZERO,
                block_number: 0,
                epoch: 0,
                validators: vec![],
            })
        } else {
            Ok(self.snapshot.clone())
        }
    }
    fn undelegate_period(&self, _at: B256) -> Result<u32, ReadError> {
        Ok(7)
    }
    fn epoch_block_interval(&self, _at: B256) -> Result<u32, ReadError> {
        Ok(100)
    }
    fn dpos_activation_block(&self, _at: B256) -> Result<u64, ReadError> {
        Ok(0)
    }
    fn active_registry_peers(&self, _at: B256) -> Result<Vec<PeerPubkey>, ReadError> {
        Ok(vec![])
    }
}

#[derive(Default)]
struct StubFallback {
    snapshot: Option<ValidatorSetSnapshot>,
}

impl StaleEpochFallback for StubFallback {
    fn get_by_epoch<'a>(
        &'a self,
        _epoch: u64,
    ) -> std::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<Option<ValidatorSetSnapshot>, ReadError>>
                + Send
                + 'a,
        >,
    > {
        let snap = self.snapshot.clone();
        Box::pin(async move { Ok(snap) })
    }
}

#[derive(Default)]
struct RecordedCall {
    target: Address,
    calldata: Vec<u8>,
}

#[derive(Clone)]
struct RecordingSink {
    calls: Arc<TokioMutex<Vec<RecordedCall>>>,
    outcome: SubmitOutcomeKind,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum SubmitOutcomeKind {
    Mined,
    AlreadySlashed,
    Failed,
}

impl SlasherTxSink for RecordingSink {
    fn submit<'a>(
        &'a self,
        target: Address,
        calldata: alloy_primitives::Bytes,
    ) -> std::pin::Pin<Box<dyn core::future::Future<Output = SubmitOutcome> + Send + 'a>> {
        let outcome_kind = self.outcome;
        Box::pin(async move {
            let mut calls = self.calls.lock().await;
            calls.push(RecordedCall {
                target,
                calldata: calldata.to_vec(),
            });
            match outcome_kind {
                SubmitOutcomeKind::Mined => SubmitOutcome::Mined {
                    tx_hash: B256::repeat_byte(0xCC),
                },
                SubmitOutcomeKind::AlreadySlashed => SubmitOutcome::AlreadySlashed,
                SubmitOutcomeKind::Failed => SubmitOutcome::Failed("stub-failure".into()),
            }
        })
    }
}

/// Counting Reporter — stands in for the marshal mailbox arm of the
/// multiplex. Marshal drops `Conflicting*` events; this stub
/// just counts everything so we can prove the multiplex fanned out.
#[derive(Clone)]
struct CountingReporter {
    tx: mpsc::UnboundedSender<()>,
}

impl Reporter for CountingReporter {
    type Activity = Activity<BlsScheme, fluentbase_consensus::Digest>;

    async fn report(&mut self, _: Self::Activity) {
        let _ = self.tx.send(());
    }
}

#[test]
fn reporter_multiplex_routes_conflicting_notarize_to_slasher() {
    let (ev_sha256, bimap) = build_conflicting_notarize();
    let test_committee = fluentbase_bls::EpochCommittee::from_unverified(EPOCH, bimap);
    let args = extract_from_conflicting_notarize(&ev_sha256, &test_committee)
        .expect("extract works on synthetic");
    assert_eq!(args.kind, SlashKind::ConflictingNotarize);
    assert!((ev_sha256.signer().get() as usize) < COMMITTEE_N);

    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|_ctx| async move {
        let (slash_tx, mut slash_rx) = mpsc::unbounded_channel::<slasher::ingress::Envelope>();
        let slasher_mailbox = slasher::ingress::test_only_mailbox(slash_tx);

        let (m_count_tx, mut m_count_rx) = mpsc::unbounded_channel::<()>();
        let marshal_stub = CountingReporter { tx: m_count_tx };

        let mut reporters: Reporters<
            Activity<BlsScheme, fluentbase_consensus::Digest>,
            CountingReporter,
            slasher::Mailbox,
        > = Reporters::from((marshal_stub, slasher_mailbox));

        let (ev, _kps, _bimap) = build_consensus_digest_conflicting_notarize();
        let activity: Activity<BlsScheme, fluentbase_consensus::Digest> =
            Activity::ConflictingNotarize(ev);
        reporters.report(activity).await;

        let delivered = slash_rx.try_recv().expect("slasher mailbox received");
        assert_eq!(
            delivered.provenance,
            slasher::ingress::Provenance::Engine,
            "the simplex Reporter arm is the engine path, and only it may move the epoch cursor"
        );
        m_count_rx.try_recv().expect("marshal arm received");
        assert!(slash_rx.try_recv().is_err(), "exactly 1 event");
        assert!(m_count_rx.try_recv().is_err(), "exactly 1 event");
    });
}

/// Build the slasher Actor with stub dependencies and start it under the
/// deterministic context. Returns:
/// - the mailbox sender for driving the test
/// - the recorded calls handle (read after exercising the pipeline)
/// - the charge store the actor fills, standing in for the proposer's read
/// - the actor handle for graceful shutdown
async fn spawn_actor_with_stubs(
    ctx: commonware_runtime::deterministic::Context,
    reader: StubReader,
    fallback: Arc<dyn StaleEpochFallback>,
    sink_outcome: SubmitOutcomeKind,
    partition: &str,
    // Retained in the signature (callers pass their committee) but no longer used
    // to register a scheme: pre-submit verify is vote-only from the committee (bug 4).
    _scheme_bimap: &BiMap<PeerPubkey, BlsPubkey>,
) -> (
    slasher::Mailbox,
    Arc<TokioMutex<Vec<RecordedCall>>>,
    slasher::ChargeStore,
    commonware_runtime::Handle<()>,
) {
    // These cases drive the mailbox directly; no evidence channel is wired.
    spawn_actor_with_evidence(ctx, reader, fallback, sink_outcome, partition, None).await
}

/// As [`spawn_actor_with_stubs`], but with the evidence-gossip bridge wired so a
/// case can also feed the actor peer-forwarded votes.
async fn spawn_actor_with_evidence(
    ctx: commonware_runtime::deterministic::Context,
    reader: StubReader,
    fallback: Arc<dyn StaleEpochFallback>,
    sink_outcome: SubmitOutcomeKind,
    partition: &str,
    evidence: Option<slasher::EvidenceBridge>,
) -> (
    slasher::Mailbox,
    Arc<TokioMutex<Vec<RecordedCall>>>,
    slasher::ChargeStore,
    commonware_runtime::Handle<()>,
) {
    use commonware_runtime::Metrics as _;
    let staking_address = Address::repeat_byte(0xEE);
    let sink_calls = Arc::new(TokioMutex::new(Vec::<RecordedCall>::new()));
    let sink: Arc<dyn SlasherTxSink> = Arc::new(RecordingSink {
        calls: sink_calls.clone(),
        outcome: sink_outcome,
    });

    // Initialize the WAL under the deterministic runtime.
    let (wal_writer, wal_reader) =
        slasher::actor::init_wal_queue(ctx.with_label("wal"), partition.into())
            .await
            .expect("queue::shared::init under deterministic runtime");

    // The slasher pre-submit verify is ALWAYS vote-only now (bug 4), rebuilding a
    // `VoteScheme::verifier` from the recovered committee — no scheme provider.

    let latest: slasher::actor::LatestFinalizedHash = Arc::new(|| Some(B256::ZERO));
    // No proposer here, so nothing drains what the actor holds for a block —
    // the epoch boundary is the only thing that empties it.
    let charges = slasher::ChargeStore::default();
    let cfg = slasher::actor::Config {
        staking_address,
        chain_id: C_MAIN,
        reader,
        latest_finalized_hash: latest,
        stale_fallback: fallback,
        sink,
        wal_writer,
        wal_reader,
        evidence,
        charges: charges.clone(),
    };
    let (actor, mailbox) = slasher::Actor::init(ctx.with_label("slasher"), cfg);
    let handle = actor.start();
    (mailbox, sink_calls, charges, handle)
}

/// Drive `activity`, then an activity in the next epoch.
///
/// The epoch turn is the event that hands a charge to the transaction
/// fallback: while its own epoch runs, a charge is held for a proposer to put
/// in a block, and only members of that epoch's committee could verify it
/// there — so once the epoch ends the transaction is the only route left.
async fn report_and_close_epoch(
    mb: &mut slasher::Mailbox,
    activity: Activity<BlsScheme, fluentbase_consensus::Digest>,
) {
    use commonware_consensus::Reporter as _;
    mb.report(activity).await;
    mb.report(offender_notarize(EPOCH + 1, 1, 0x11)).await;
}

/// Helper: drive a `ConflictingNotarize` event into the mailbox and wait
/// (bounded by ~100 polling iterations) for the sink to record at least
/// `n` calls.
async fn wait_for_sink_calls(calls: &Arc<TokioMutex<Vec<RecordedCall>>>, n: usize) -> bool {
    for _ in 0..200 {
        let len = { calls.lock().await.len() };
        if len >= n {
            return true;
        }
        // Yield to the runtime so the consumer task can advance.
        tokio::task::yield_now().await;
    }
    false
}

/// The Phase-6 property: a charge that never reaches a block before its epoch
/// ends is not lost — the epoch turn hands it to the transaction route.
#[test]
fn a_charge_stranded_by_the_epoch_boundary_lands_by_transaction() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (_kps_unused, bimap) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (ev, _kps, _bimap_d) = build_consensus_digest_conflicting_notarize();
        let (mailbox, calls, charges, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_boundary_fallback",
            &bimap,
        )
        .await;

        use commonware_consensus::Reporter as _;
        let mut mb = mailbox;
        mb.report(Activity::ConflictingNotarize(ev)).await;
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        assert!(
            charges.next_charge(EPOCH, |_| false).is_some(),
            "inside its own epoch the charge is held for a proposer"
        );
        assert_eq!(
            calls.lock().await.len(),
            0,
            "and is NOT spent on a transaction while the block route is open"
        );

        mb.report(offender_notarize(EPOCH + 1, 1, 0x11)).await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "the epoch turn must hand the stranded charge to the sink"
        );
        let recorded = calls.lock().await;
        assert_eq!(recorded.len(), 1, "exactly one sink.submit call");
        assert_eq!(
            recorded[0].target,
            Address::repeat_byte(0xEE),
            "sink called with the configured staking address"
        );
        // The calldata must be a `slashEquivocationNotarize` call — confirm the
        // exact ABI selector (not merely "len >= 4"), against the literal pin.
        assert_eq!(
            recorded[0].calldata[..4],
            slash_abi::SEL_NOTARIZE,
            "calldata ABI selector must be slashEquivocationNotarize (0xe28d2f63)"
        );
        assert!(
            charges.next_charge(EPOCH, |_| false).is_none(),
            "a drained charge is released, so no later turn re-submits it"
        );

        drop(recorded);
        // Cleanup: drop the mailbox so producer + consumer exit cleanly.
        drop(mb);
        handle.abort();
    });
}

/// The other half of the one-epoch grace [`slasher::ChargeStore`]'s vote store
/// keeps: a half that arrives after the boundary still pairs, and the charge it
/// makes is for an epoch whose drain has already run. It must go straight to
/// the sink rather than joining a queue nothing will empty again.
#[test]
fn a_charge_assembled_after_the_boundary_goes_straight_to_the_sink() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (_kps, bimap) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (mailbox, calls, charges, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_late_match",
            &bimap,
        )
        .await;

        use commonware_consensus::Reporter as _;
        let mut mb = mailbox;
        // The epoch turns FIRST, with nothing queued for the old one.
        mb.report(offender_notarize(EPOCH + 1, 1, 0x11)).await;
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            calls.lock().await.len(),
            0,
            "an epoch turn with an empty queue submits nothing"
        );

        // Only now do the two halves of an EPOCH equivocation meet.
        mb.report(offender_notarize(EPOCH, VIEW, 0xaa)).await;
        mb.report(offender_notarize(EPOCH, VIEW, 0xbb)).await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "a charge assembled after the boundary must still reach the sink"
        );
        assert_eq!(
            calls.lock().await[0].calldata[..4],
            slash_abi::SEL_NOTARIZE,
            "calldata ABI selector must be slashEquivocationNotarize (0xe28d2f63)"
        );
        assert!(
            charges.next_charge(EPOCH, |_| false).is_none(),
            "and must not sit in the queue, whose drain for EPOCH already ran"
        );

        drop(mb);
        handle.abort();
    });
}

/// One signed notarize by OFFENDER as a wire `Vote` — what a peer forwards on
/// the evidence channel.
fn offender_vote(epoch: u64, view: u64, tag: u8) -> Vote<BlsScheme, fluentbase_consensus::Digest> {
    match offender_notarize(epoch, view, tag) {
        Activity::Notarize(n) => Vote::Notarize(n),
        _ => unreachable!("offender_notarize builds a Notarize"),
    }
}

/// Resolver over the test committee, standing in for the node's chain read.
fn evidence_committee_for(bimap: &BiMap<PeerPubkey, BlsPubkey>) -> slasher::EvidenceCommitteeFor {
    let bimap = bimap.clone();
    Arc::new(move |epoch: u64| {
        Some(fluentbase_bls::EpochCommittee::from_unverified(
            epoch,
            bimap.clone(),
        ))
    })
}

async fn settle() {
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
}

/// Gossip may add votes; it must never move the epoch cursor.
///
/// A member of the committee for `EPOCH + 2` — already committed on chain under
/// the two-epoch ahead-commit horizon — signs one genuine `Notarize` for a round
/// in its own future epoch and publishes it. Signing is not bound to the live
/// view, so the signature is real. If that were allowed to set the cursor, every
/// receiving node would prune its vote store past the live epoch and push every
/// live charge onto the transaction route, at one Byzantine member's discretion
/// and permanently, since the cursor is monotone.
#[test]
fn a_gossiped_vote_naming_a_future_epoch_neither_moves_the_cursor_nor_flushes_the_store() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (_kps, bimap) = committee(1);
        let reader = StubReader {
            snapshot: snapshot_from_bimap(&bimap),
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (bridge, _publications) = slasher::EvidenceBridge::new();
        let (mailbox, calls, charges, handle) = spawn_actor_with_evidence(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_gossip_cursor",
            Some(bridge.clone()),
        )
        .await;

        use commonware_consensus::Reporter as _;
        let mut mb = mailbox;
        // The local engine is at EPOCH, and this node holds one half of a
        // split-delivered equivocation for it.
        mb.report(offender_notarize(EPOCH, VIEW, 0xaa)).await;
        settle().await;
        assert_eq!(
            bridge.epoch_cursor().get(),
            EPOCH,
            "the engine sets the cursor"
        );

        // The Byzantine publication. It reaches the vote store as gossip — that
        // is allowed — but says nothing about where consensus is.
        bridge
            .gossip_sink()
            .expect("the actor bound its sink at init")
            .report_gossiped(offender_notarize(EPOCH + 2, 1, 0x55));
        settle().await;
        assert_eq!(
            bridge.epoch_cursor().get(),
            EPOCH,
            "a forwarded vote must not move the epoch cursor"
        );

        // The other half arrives. It can only pair if the first half survived,
        // which is what a cursor jump would have destroyed.
        mb.report(offender_notarize(EPOCH, VIEW, 0xbb)).await;
        settle().await;
        assert!(
            charges.next_charge(EPOCH, |_| false).is_some(),
            "the held half survived, so the pair still assembles into a block charge"
        );
        assert_eq!(
            calls.lock().await.len(),
            0,
            "and the charge is still on the in-block route, not flushed to transactions"
        );

        drop(mb);
        handle.abort();
    });
}

/// The bound rejects a claim, not the feature: a forwarded vote inside the
/// retained window still lands in the store and still pairs into a charge. This
/// is the whole reason the channel exists — without it a cleanly split
/// equivocation leaves no node holding both halves.
#[test]
fn a_gossiped_vote_inside_the_window_still_assembles_a_charge() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (_kps, bimap) = committee(1);
        let reader = StubReader {
            snapshot: snapshot_from_bimap(&bimap),
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (bridge, _publications) = slasher::EvidenceBridge::new();
        let (mailbox, calls, charges, handle) = spawn_actor_with_evidence(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_gossip_in_window",
            Some(bridge.clone()),
        )
        .await;

        use commonware_consensus::Reporter as _;
        let mut mb = mailbox;
        // This node was shown only one half by the equivocator.
        mb.report(offender_notarize(EPOCH, VIEW, 0xaa)).await;
        settle().await;
        assert!(
            charges.next_charge(EPOCH, |_| false).is_none(),
            "one half is not evidence"
        );

        // A peer that was shown the other half republishes it, through the full
        // inbound path: decode, epoch bound, committee resolve, signature verify.
        let batch = slasher::gossip::encode_batch(&vec![offender_vote(EPOCH, VIEW, 0xbb)]);
        slasher::gossip::ingest_batch(
            batch.as_ref(),
            C_MAIN,
            &evidence_committee_for(&bimap),
            &bridge,
        );
        settle().await;

        assert!(
            charges.next_charge(EPOCH, |_| false).is_some(),
            "the forwarded half pairs with the held one into a block charge"
        );
        assert_eq!(
            calls.lock().await.len(),
            0,
            "inside its own epoch the charge takes the block route"
        );

        drop(mb);
        handle.abort();
    });
}

#[test]
fn slasher_falls_back_to_cache_on_empty_snapshot() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (kps, bimap) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap);
        // Reader returns empty; fallback returns the real snapshot.
        let reader = StubReader {
            snapshot: snapshot.clone(),
            empty: true,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback {
            snapshot: Some(snapshot),
        });

        let (ev, _kps_unused, _bimap_d) = build_consensus_digest_conflicting_notarize();
        let _kps_keep = kps;
        let (mailbox, calls, _charges, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_h17_fallback",
            &bimap,
        )
        .await;

        let mut mb = mailbox;
        report_and_close_epoch(&mut mb, Activity::ConflictingNotarize(ev)).await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "fallback path should still result in a sink.submit call"
        );

        drop(mb);
        handle.abort();
    });
}

#[test]
fn slasher_rejects_tampered_evidence_at_verify_pre_submit() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        // The snapshot committee (committee 2) does NOT match the committee that
        // SIGNED the evidence (committee 1). Pre-submit rebuilds the vote verifier
        // from the SNAPSHOT committee (bug 4 — always vote-only from the recovered
        // committee), so the evidence's signatures fail to validate against it and
        // are rejected before reaching the sink.
        let (_kps_b, bimap_b) = committee(2);
        let snapshot = snapshot_from_bimap(&bimap_b);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());

        // Evidence signed by committee 1 (a DIFFERENT committee from the snapshot).
        let (ev, _kps_unused, _bimap_d) = build_consensus_digest_conflicting_notarize();

        let (mailbox, calls, _charges, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_verify_reject",
            &bimap_b,
        )
        .await;

        let mut mb = mailbox;
        report_and_close_epoch(&mut mb, Activity::ConflictingNotarize(ev)).await;

        // Give the actor a chance to run; it should NOT have enqueued
        // anything — the verify gate refused the charge, so the epoch turn
        // finds nothing queued to hand on.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let recorded = calls.lock().await;
        assert_eq!(
            recorded.len(),
            0,
            "tampered evidence must not reach the sink (verify_pre_submit gate)"
        );

        drop(mb);
        handle.abort();
    });
}

// ---- slasher coverage: ABI selectors, dedup / outcome-lifecycle, kind coverage ----

/// Literal 4-byte selectors of the three slash entry points, used to assert
/// which slash function the producer encoded.
///
/// **Hardcoded on purpose.** This file used to carry a private `sol!` mirror of
/// the production declaration and compare `SomeCall::SELECTOR` against calldata
/// the production declaration produced — two copies of the same belief, which
/// agree through any rename. These literals were computed with
/// `cast sig "<signature>"` and are the independent half of the pin.
///
/// The contract-side counterparts differ (six arguments, not four); the merge
/// checklist is at the top of `crates/dpos/consensus/src/slasher/actor.rs`.
mod slash_abi {
    /// `cast sig "slashEquivocationNotarize(bytes,bytes,bytes,bytes)"`
    pub const SEL_NOTARIZE: [u8; 4] = [0xe2, 0x8d, 0x2f, 0x63];
    /// `cast sig "slashEquivocationFinalize(bytes,bytes,bytes,bytes)"`
    pub const SEL_FINALIZE: [u8; 4] = [0xad, 0xd0, 0x7a, 0x3e];
    /// `cast sig "slashEquivocationNullifyFinalize(bytes,bytes,bytes,bytes)"`
    pub const SEL_NULLIFY_FINALIZE: [u8; 4] = [0xa1, 0x08, 0x27, 0xe9];
}

/// The production `sol!` in `slasher::actor` — the ONE declaration the producer
/// encodes through — against the literal selectors above. A rename on either
/// side fails here first, and the message names both sides.
#[test]
fn slash_abi_selectors_are_pinned() {
    use alloy_sol_types::SolCall as _;
    use fluentbase_consensus::slasher::actor::{
        slashEquivocationFinalizeCall, slashEquivocationNotarizeCall,
        slashEquivocationNullifyFinalizeCall,
    };

    assert_eq!(
        slashEquivocationNotarizeCall::SELECTOR,
        slash_abi::SEL_NOTARIZE,
        "node emits {:?} for slashEquivocationNotarize; pinned literal is 0xe28d2f63 and the \
         contract (feat/flu-989-port-solidity-delta, six args) is 0x2bc5fb10",
        slashEquivocationNotarizeCall::SELECTOR
    );
    assert_eq!(
        slashEquivocationFinalizeCall::SELECTOR,
        slash_abi::SEL_FINALIZE,
        "node emits {:?} for slashEquivocationFinalize; pinned literal is 0xadd07a3e and the \
         contract (feat/flu-989-port-solidity-delta, six args) is 0xb034c58b",
        slashEquivocationFinalizeCall::SELECTOR
    );
    assert_eq!(
        slashEquivocationNullifyFinalizeCall::SELECTOR,
        slash_abi::SEL_NULLIFY_FINALIZE,
        "node emits {:?} for slashEquivocationNullifyFinalize; pinned literal is 0xa10827e9 and \
         the contract (feat/flu-989-port-solidity-delta, six args) is 0x337e1437",
        slashEquivocationNullifyFinalizeCall::SELECTOR
    );
}

/// A `ConflictingFinalize` by OFFENDER (two finalizes, same round, differing
/// proposals) over consensus digests.
fn build_conflicting_finalize() -> ConflictingFinalize<BlsScheme, fluentbase_consensus::Digest> {
    let (kps, bimap) = committee(1);
    let s = build_signer(&fluent_namespace(C_MAIN), bimap, &kps[OFFENDER], None)
        .expect("offender in committee");
    let round = Round::new(Epoch::new(EPOCH), View::new(VIEW));
    let p1 = Proposal::new(
        round,
        View::new(VIEW - 1),
        fluentbase_consensus::Digest(B256::from([0xaa; 32])),
    );
    let p2 = Proposal::new(
        round,
        View::new(VIEW - 1),
        fluentbase_consensus::Digest(B256::from([0xbb; 32])),
    );
    let f1 = Finalize::sign(&s, p1).expect("sign f1");
    let f2 = Finalize::sign(&s, p2).expect("sign f2");
    ConflictingFinalize::new(f1, f2)
}

/// A `NullifyFinalize` by OFFENDER (nullify + finalize for the same round).
fn build_nullify_finalize() -> NullifyFinalize<BlsScheme, fluentbase_consensus::Digest> {
    let (kps, bimap) = committee(1);
    let s = build_signer(&fluent_namespace(C_MAIN), bimap, &kps[OFFENDER], None)
        .expect("offender in committee");
    let round = Round::new(Epoch::new(EPOCH), View::new(VIEW));
    let nullify = Nullify::sign::<fluentbase_consensus::Digest>(&s, round).expect("sign nullify");
    let p = Proposal::new(
        round,
        View::new(VIEW - 1),
        fluentbase_consensus::Digest(B256::from([0xcc; 32])),
    );
    let finalize = Finalize::sign(&s, p).expect("sign finalize");
    NullifyFinalize::new(nullify, finalize)
}

/// Drive two same-victim events with the given sink outcome; assert how many
/// sink calls result (1 = victim deduped after the first; 2 = not deduped).
/// This exercises the consumer's outcome lifecycle: `Mined`/`AlreadySlashed`
/// insert the victim into the in-session dedup set, `Failed` does not.
///
/// The first event takes the boundary drain, the second the late-match route —
/// both end at the same per-victim dedup, which is the point.
fn dedup_call_count(outcome: SubmitOutcomeKind, partition: &'static str) -> usize {
    use std::sync::{Arc as StdArc, Mutex as StdMutex};
    let observed = StdArc::new(StdMutex::new(0usize));
    let observed_w = observed.clone();
    let partition = partition.to_string();
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(move |ctx| async move {
        let (_kps, bimap) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (mailbox, calls, _charges, handle) =
            spawn_actor_with_stubs(ctx.clone(), reader, fallback, outcome, &partition, &bimap)
                .await;
        let mut mb = mailbox;
        use commonware_consensus::Reporter as _;

        // First event → exactly one sink call; on Mined/AlreadySlashed the
        // consumer then inserts the victim into the dedup set.
        let (ev1, _k, _b) = build_consensus_digest_conflicting_notarize();
        report_and_close_epoch(&mut mb, Activity::ConflictingNotarize(ev1)).await;
        assert!(wait_for_sink_calls(&calls, 1).await, "first submit");
        // Let the consumer finish the post-submit insert + ack.
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }

        // Second event, SAME offender/victim.
        let (ev2, _k2, _b2) = build_consensus_digest_conflicting_notarize();
        mb.report(Activity::ConflictingNotarize(ev2)).await;
        for _ in 0..300 {
            tokio::task::yield_now().await;
        }

        *observed_w.lock().unwrap() = calls.lock().await.len();
        drop(mb);
        handle.abort();
    });
    let n = *observed.lock().unwrap();
    n
}

#[test]
fn slasher_dedup_skips_already_submitted_victim() {
    // Mined inserts the victim → the second same-victim event is deduped.
    assert_eq!(
        dedup_call_count(SubmitOutcomeKind::Mined, "slasher_dedup_mined"),
        1,
        "second same-victim event must be deduped after a Mined outcome"
    );
}

#[test]
fn slasher_already_slashed_dedups_victim() {
    // AlreadySlashed (pre-flight tombstoned) also inserts the victim → deduped.
    assert_eq!(
        dedup_call_count(SubmitOutcomeKind::AlreadySlashed, "slasher_dedup_already"),
        1,
        "AlreadySlashed must insert the victim into the dedup set"
    );
}

#[test]
fn slasher_failed_outcome_does_not_dedup_victim() {
    // Failed does NOT insert the victim → the second event is submitted again.
    assert_eq!(
        dedup_call_count(SubmitOutcomeKind::Failed, "slasher_dedup_failed"),
        2,
        "Failed must NOT dedup — the victim is retried (submitted again)"
    );
}

#[test]
fn slasher_pipeline_handles_conflicting_finalize() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (_kps, bimap) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (mailbox, calls, _charges, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_conflicting_finalize",
            &bimap,
        )
        .await;
        let mut mb = mailbox;
        report_and_close_epoch(
            &mut mb,
            Activity::ConflictingFinalize(build_conflicting_finalize()),
        )
        .await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "ConflictingFinalize must flow through to the sink"
        );
        let recorded = calls.lock().await;
        assert_eq!(
            recorded[0].calldata[..4],
            slash_abi::SEL_FINALIZE,
            "ABI selector must be slashEquivocationFinalize (0xadd07a3e)"
        );
        drop(recorded);
        drop(mb);
        handle.abort();
    });
}

#[test]
fn slasher_pipeline_handles_nullify_finalize() {
    let runtime = commonware_runtime::deterministic::Runner::default();
    runtime.start(|ctx| async move {
        let (_kps, bimap) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());
        let (mailbox, calls, _charges, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_nullify_finalize",
            &bimap,
        )
        .await;
        let mut mb = mailbox;
        report_and_close_epoch(&mut mb, Activity::NullifyFinalize(build_nullify_finalize())).await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "NullifyFinalize must flow through to the sink"
        );
        let recorded = calls.lock().await;
        assert_eq!(
            recorded[0].calldata[..4],
            slash_abi::SEL_NULLIFY_FINALIZE,
            "ABI selector must be slashEquivocationNullifyFinalize (0xa10827e9)"
        );
        drop(recorded);
        drop(mb);
        handle.abort();
    });
}
