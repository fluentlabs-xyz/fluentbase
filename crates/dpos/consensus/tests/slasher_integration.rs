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
//!    stub. Asserts:
//!    - `slasher_full_pipeline_records_wal_then_submits_via_sink` —
//!      mailbox event → WAL enqueue → sink.submit called with the right
//!      calldata.
//!    - `slasher_falls_back_to_cache_on_empty_snapshot`.
//!    - `slasher_rejects_tampered_evidence_at_verify_pre_submit` —
//!      pre-submit fails → no sink call.
//!    - `slasher_dedup_skips_already_submitted_victim`.

use alloy_primitives::{Address, B256};
use alloy_sol_types::SolCall as _;
use commonware_codec::DecodeExt;
use commonware_consensus::{
    simplex::types::{
        Activity, Attributable, ConflictingFinalize, ConflictingNotarize, Finalize, Notarize,
        Nullify, NullifyFinalize, Proposal,
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
use fluentbase_consensus::{
    outer::EpochSchemeProvider,
    slasher::{
        self,
        actor::{SlasherTxSink, StaleEpochFallback, SubmitOutcome},
        evidence::{extract_from_conflicting_notarize, SlashKind},
    },
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
    let s = build_signer(&fluent_namespace(C_MAIN), bimap.clone(), &kps[OFFENDER])
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
    let s = build_signer(&fluent_namespace(C_MAIN), bimap.clone(), &kps[OFFENDER])
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
        let (slash_tx, mut slash_rx) =
            mpsc::unbounded_channel::<Activity<BlsScheme, fluentbase_consensus::Digest>>();
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

        slash_rx.try_recv().expect("slasher mailbox received");
        m_count_rx.try_recv().expect("marshal arm received");
        assert!(slash_rx.try_recv().is_err(), "exactly 1 event");
        assert!(m_count_rx.try_recv().is_err(), "exactly 1 event");
    });
}

/// Build the slasher Actor with stub dependencies and start it under the
/// deterministic context. Returns:
/// - the mailbox sender for driving the test
/// - the recorded calls handle (read after exercising the pipeline)
/// - the actor handle for graceful shutdown
async fn spawn_actor_with_stubs(
    ctx: commonware_runtime::deterministic::Context,
    reader: StubReader,
    fallback: Arc<dyn StaleEpochFallback>,
    sink_outcome: SubmitOutcomeKind,
    partition: &str,
    scheme_bimap: &BiMap<PeerPubkey, BlsPubkey>,
) -> (
    slasher::Mailbox,
    Arc<TokioMutex<Vec<RecordedCall>>>,
    commonware_runtime::Handle<()>,
) {
    use commonware_runtime::Metrics as _;
    use fluentbase_bls::scheme::build_verifier;
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

    // Register a verifier-mode scheme for EPOCH built from `scheme_bimap`.
    // The slasher only needs verify_pre_submit's verification path, not
    // signing — `build_verifier` is sufficient and accepts any committee.
    let scheme_provider = EpochSchemeProvider::new();
    let scheme = build_verifier(&fluent_namespace(C_MAIN), scheme_bimap.clone());
    scheme_provider.register(Epoch::new(EPOCH), scheme);

    let latest: slasher::actor::LatestFinalizedHash = Arc::new(|| Some(B256::ZERO));
    let cfg = slasher::actor::Config {
        staking_address,
        reader,
        latest_finalized_hash: latest,
        scheme_provider,
        stale_fallback: fallback,
        sink,
        wal_writer,
        wal_reader,
    };
    let (actor, mailbox) = slasher::Actor::init(ctx.with_label("slasher"), cfg);
    let handle = actor.start();
    (mailbox, sink_calls, handle)
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

#[test]
fn slasher_full_pipeline_records_wal_then_submits_via_sink() {
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
        let (mailbox, calls, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_full_pipeline",
            &bimap,
        )
        .await;

        // Drive the event through the mailbox.
        use commonware_consensus::Reporter as _;
        let mut mb = mailbox;
        mb.report(Activity::ConflictingNotarize(ev)).await;

        // The consumer should pick up the WAL entry and call the sink.
        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "sink.submit was not called within timeout"
        );
        let recorded = calls.lock().await;
        assert_eq!(recorded.len(), 1, "exactly one sink.submit call");
        assert_eq!(
            recorded[0].target,
            Address::repeat_byte(0xEE),
            "sink called with the configured staking address"
        );
        // The calldata must be a `slashEquivocationNotarize` call — confirm the
        // exact ABI selector (not merely "len >= 4").
        assert_eq!(
            &recorded[0].calldata[..4],
            slash_abi::slashEquivocationNotarizeCall::SELECTOR.as_slice(),
            "calldata ABI selector must be slashEquivocationNotarize"
        );

        // Cleanup: drop the mailbox so producer + consumer exit cleanly.
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
        let (mailbox, calls, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_h17_fallback",
            &bimap,
        )
        .await;

        let mut mb = mailbox;
        use commonware_consensus::Reporter as _;
        mb.report(Activity::ConflictingNotarize(ev)).await;

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
        // Build a snapshot for one committee but register the SCHEME for a
        // DIFFERENT committee. `verify_pre_submit` will reject because the
        // signature pubkeys won't align with the registered scheme.
        let (kps_a, bimap_a) = committee(1);
        let snapshot = snapshot_from_bimap(&bimap_a);
        let reader = StubReader {
            snapshot,
            empty: false,
        };
        let fallback: Arc<dyn StaleEpochFallback> = Arc::new(StubFallback::default());

        // Sign the evidence with kps_a, but register a verifier scheme
        // built from a DIFFERENT committee so verify_pre_submit rejects
        // (signature pubkeys won't validate against the registered scheme's
        // participant set).
        let (ev, _kps_unused, _bimap_d) = build_consensus_digest_conflicting_notarize();
        let (_kps_b, bimap_b) = committee(2);
        let _ = (kps_a, &bimap_a); // ensure original committee borrows stay alive

        let (mailbox, calls, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_verify_reject",
            &bimap_b, // Mismatched scheme committee → verify_pre_submit fails
        )
        .await;

        let mut mb = mailbox;
        use commonware_consensus::Reporter as _;
        mb.report(Activity::ConflictingNotarize(ev)).await;

        // Give the actor a chance to run; it should NOT have enqueued
        // anything because verify_pre_submit failed in the producer.
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

/// Local ABI mirror of the three slash entry points (types only — the selector
/// depends solely on `(bytes,bytes,bytes,bytes)`), used to assert which slash
/// function the producer encoded.
mod slash_abi {
    alloy_sol_types::sol! {
        function slashEquivocationNotarize(bytes evidence, bytes pk, bytes sig1, bytes sig2);
        function slashEquivocationFinalize(bytes evidence, bytes pk, bytes sig1, bytes sig2);
        function slashEquivocationNullifyFinalize(bytes evidence, bytes pk, bytes sig1, bytes sig2);
    }
}

/// A `ConflictingFinalize` by OFFENDER (two finalizes, same round, differing
/// proposals) over consensus digests.
fn build_conflicting_finalize() -> ConflictingFinalize<BlsScheme, fluentbase_consensus::Digest> {
    let (kps, bimap) = committee(1);
    let s = build_signer(&fluent_namespace(C_MAIN), bimap, &kps[OFFENDER])
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
    let s = build_signer(&fluent_namespace(C_MAIN), bimap, &kps[OFFENDER])
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
        let (mailbox, calls, handle) =
            spawn_actor_with_stubs(ctx.clone(), reader, fallback, outcome, &partition, &bimap)
                .await;
        let mut mb = mailbox;
        use commonware_consensus::Reporter as _;

        // First event → exactly one sink call; on Mined/AlreadySlashed the
        // consumer then inserts the victim into the dedup set.
        let (ev1, _k, _b) = build_consensus_digest_conflicting_notarize();
        mb.report(Activity::ConflictingNotarize(ev1)).await;
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
        let (mailbox, calls, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_conflicting_finalize",
            &bimap,
        )
        .await;
        let mut mb = mailbox;
        use commonware_consensus::Reporter as _;
        mb.report(Activity::ConflictingFinalize(build_conflicting_finalize()))
            .await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "ConflictingFinalize must flow through to the sink"
        );
        let recorded = calls.lock().await;
        assert_eq!(
            &recorded[0].calldata[..4],
            slash_abi::slashEquivocationFinalizeCall::SELECTOR.as_slice(),
            "ABI selector must be slashEquivocationFinalize"
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
        let (mailbox, calls, handle) = spawn_actor_with_stubs(
            ctx.clone(),
            reader,
            fallback,
            SubmitOutcomeKind::Mined,
            "slasher_nullify_finalize",
            &bimap,
        )
        .await;
        let mut mb = mailbox;
        use commonware_consensus::Reporter as _;
        mb.report(Activity::NullifyFinalize(build_nullify_finalize()))
            .await;

        assert!(
            wait_for_sink_calls(&calls, 1).await,
            "NullifyFinalize must flow through to the sink"
        );
        let recorded = calls.lock().await;
        assert_eq!(
            &recorded[0].calldata[..4],
            slash_abi::slashEquivocationNullifyFinalizeCall::SELECTOR.as_slice(),
            "ABI selector must be slashEquivocationNullifyFinalize"
        );
        drop(recorded);
        drop(mb);
        handle.abort();
    });
}
