//! Slasher actor: per-Activity filter → committee resolve → extract →
//! ABI-encode → durable WAL → consumer task submits to txpool → ack.
//!
//! The actor is split into a producer (mailbox loop in `run_producer`)
//! and a consumer
//! (`run_consumer`) task connected through `commonware_storage::queue::shared`.
//! Producer enqueues `(victim || calldata)` blobs after `verify_pre_submit`;
//! consumer dequeues, hands them to the [`SlasherTxSink`], and acks the
//! queue on Mined / AlreadySlashed (goal achieved on-chain) or leaves the
//! entry un-acked on submission failure. NOTE: an un-acked entry is
//! re-delivered ONLY after a process restart — `recv` advances `read_pos`
//! unconditionally, so there is no automatic in-session retry.

use super::evidence::{
    extract_from_conflicting_finalize, extract_from_conflicting_notarize,
    extract_from_nullify_finalize, verify_pre_submit, verify_pre_submit_vote_only, SlashCallArgs,
    SlashKind,
};
use crate::{
    digest::Digest,
    scheme::epoch_committee_from_snapshot,
    slasher::ingress::{Mailbox, Message},
};
use alloy_primitives::{Address, Bytes, B256};
use alloy_sol_types::SolCall;
use commonware_consensus::{
    simplex::types::{Activity, Attributable},
    types::Epoch as ConsensusEpoch,
    Epochable,
};
use commonware_cryptography::certificate::Provider as CertProvider;
use commonware_runtime::{spawn_cell, Clock, ContextCell, Handle, Metrics, Spawner, Storage};
use commonware_storage::queue::shared as wal_queue;
use fluentbase_bls::Scheme as BlsScheme;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{mpsc, Mutex as TokioMutex};
use tracing::{debug, error, info, instrument, warn};

// Solidity ABI bindings for the three slash entry points.
alloy_sol_types::sol! {
    function slashEquivocationNotarize(bytes evidence, bytes pkUncompressed,
        bytes sig1Uncompressed, bytes sig2Uncompressed) external;
    function slashEquivocationFinalize(bytes evidence, bytes pkUncompressed,
        bytes sig1Uncompressed, bytes sig2Uncompressed) external;
    function slashEquivocationNullifyFinalize(bytes evidence, bytes pkUncompressed,
        bytes sig1Uncompressed, bytes sig2Uncompressed) external;
}

// The slasher consumes `StakingStateRead` re-exported from
// `fluentbase-staking-reader`. The blanket impl on
// `RethStakingStateReader<P, E>` in
// `crates/staking-reader/src/epoch_transition.rs` provides the production impl.
use fluentbase_staking_reader::StakingStateRead;

/// Closure returning the latest finalized block hash (or `None` if not yet
/// known). Threaded in from `dpos.rs`, wraps the reth provider.
pub type LatestFinalizedHash = Arc<dyn Fn() -> Option<B256> + Send + Sync>;

/// Backoff between producer retries of a transient `handle` failure.
const SLASHER_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_secs(2);
/// Max producer attempts for one Activity before it is dropped (a missing
/// committee/scheme/finalized-hash at startup resolves within seconds; a
/// dependency that is still down after this bound is an operational failure
/// surfaced by the drop log + metric, not a silent loss on the first hiccup).
const SLASHER_MAX_RETRIES: u32 = 30;

/// Outcome of [`Actor::handle`] classifying a failure by retry-ability.
/// TRANSIENT failures (startup races: no finalized hash, RPC/state read error,
/// scheme/committee not yet registered, storage hiccup) are re-attempted by the
/// producer; PERMANENT failures (malformed/variant-mismatch evidence, BiMap
/// divergence, prune-pruned uncached committee) are dropped — retrying the same
/// bytes can never succeed.
enum HandleError {
    Transient(eyre::Report),
    Permanent(eyre::Report),
}

impl HandleError {
    fn transient(msg: impl Into<String>) -> Self {
        Self::Transient(eyre::eyre!("{}", msg.into()))
    }
    fn permanent(msg: impl Into<String>) -> Self {
        Self::Permanent(eyre::eyre!("{}", msg.into()))
    }
}

/// Stale-epoch fallback abstraction.
///
/// Trait-object-friendly read view of the cache so the slasher's Config
/// doesn't need to carry the cache's storage-backend generic. The
/// production impl (in dpos.rs) wraps an
/// `Arc<tokio::sync::Mutex<fluentbase_staking_reader::ValidatorSetCache<E>>>`.
///
/// **** я не очень люблю подход где создается trait только чтобы создать тест, должен быть способ проще
pub trait StaleEpochFallback: Send + Sync + 'static {
    fn get_by_epoch<'a>(
        &'a self,
        epoch: u64,
    ) -> std::pin::Pin<
        Box<
            dyn core::future::Future<
                    Output = Result<
                        Option<fluentbase_staking_reader::reader::ValidatorSetSnapshot>,
                        fluentbase_staking_reader::error::ReadError,
                    >,
                > + Send
                + 'a,
        >,
    >;
}

/// Production wrapper: an `Arc<tokio::sync::Mutex<ValidatorSetCache<E>>>`
/// that satisfies [`StaleEpochFallback`]. The slasher reads through this
/// trait object so its `Config` doesn't need to carry the cache's
/// storage-backend generic. dpos.rs constructs this wrapper over the
/// same `Arc<Mutex<...>>` instance threaded into `EpochTransition`.
/// Storage backend usable by [`fluentbase_staking_reader::ValidatorSetCache`]
/// (`Storage + Metrics + BufferPooler`) AND shareable across the slasher's
/// async task (the `Send + Sync + 'static` the boxed `get_by_epoch` future
/// needs). Collapses the otherwise-repeated six-line bound into one name; the
/// blanket impl makes any qualifying runtime context satisfy it automatically,
/// so the `dpos.rs` construction site needs no annotation.
pub trait CacheBackend:
    commonware_runtime::Storage
    + commonware_runtime::Metrics
    + commonware_runtime::BufferPooler
    + Send
    + Sync
    + 'static
{
}

impl<T> CacheBackend for T where
    T: commonware_runtime::Storage
        + commonware_runtime::Metrics
        + commonware_runtime::BufferPooler
        + Send
        + Sync
        + 'static
{
}

pub struct SharedCacheFallback<EStorage: CacheBackend>(
    pub Arc<TokioMutex<fluentbase_staking_reader::ValidatorSetCache<EStorage>>>,
);

impl<EStorage: CacheBackend> StaleEpochFallback for SharedCacheFallback<EStorage> {
    fn get_by_epoch<'a>(
        &'a self,
        epoch: u64,
    ) -> std::pin::Pin<
        Box<
            dyn core::future::Future<
                    Output = Result<
                        Option<fluentbase_staking_reader::reader::ValidatorSetSnapshot>,
                        fluentbase_staking_reader::error::ReadError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let cache = self.0.lock().await;
            cache.get_by_epoch(epoch).await
        })
    }
}

/// Transport abstraction over the reth `TransactionPool`. Production
/// impl in `dpos.rs` owns the slasher EOA key + `node.pool` + `node.provider`
/// and signs + submits + awaits transaction inclusion. Tests provide a
/// recording stub.
///
/// The consumer task hands `(target, calldata)` and waits for an outcome;
/// the sink is responsible for nonce management and on-chain confirmation
/// semantics (no HTTP RPC; uses `TransactionPool::add_consensus_transaction`).
/// **** я не очень люблю подход где создается trait только чтобы создать тест, должен быть способ проще
pub trait SlasherTxSink: Send + Sync + 'static {
    fn submit<'a>(
        &'a self,
        target: Address,
        calldata: Bytes,
    ) -> std::pin::Pin<Box<dyn core::future::Future<Output = SubmitOutcome> + Send + 'a>>;
}

/// Outcome categories used by the consumer to decide whether to ack the WAL
/// entry. The production sink **pre-flight-simulates** the slash call before
/// submitting (a receipt carries only a success bit, not the revert reason, so
/// revert classification must happen at simulation time):
/// - `Mined` — tx submitted AND confirmed on-chain with `status == 1`. Ack.
/// - `AlreadySlashed` — pre-flight simulation reverted with
///   `AlreadySlashedForEquivocation` (victim already tombstoned). The goal is
///   already achieved; ack without submitting a tx.
/// - `Failed` — simulation or submission failed (a deterministic encoding bug,
///   an unexpected revert, or a transient pool/inclusion error). Do NOT ack;
///   the entry is re-delivered after a process restart (NOT in-session).
///   Always paired with a loud log.
#[derive(Debug, Clone)]
pub enum SubmitOutcome {
    /// Tx submitted and confirmed on-chain with receipt `status == 1`.
    Mined { tx_hash: B256 },
    /// Pre-flight simulation showed the victim is already tombstoned
    /// (`AlreadySlashedForEquivocation`); no tx submitted. Goal achieved → ack.
    AlreadySlashed,
    /// Simulation/submission failed (bug, unexpected revert, or transient). Not
    /// acked; retried only after a process restart.
    Failed(String),
}

/// Configuration passed to [`Actor::init`]. The WAL queue handles are
/// constructed by the outer layer (via [`init_wal_queue`]) — they cannot
/// be built inside `Actor::init` because `init` is a synchronous function
/// and `queue::shared::init` is async.
pub struct Config<R, E>
where
    R: StakingStateRead + Send + Sync + 'static,
    E: Clock + Metrics + Spawner + Storage + Send + 'static,
{
    /// Staking predeploy address.
    pub staking_address: Address,
    /// L2 chain id — used to rebuild a verifier scheme (`fluent_namespace`) for
    /// an evidence epoch whose scheme the provider has pruned but whose
    /// committee the stale fallback still holds (§14).
    pub chain_id: u64,
    /// Reader for committee resolution (dedicated instance, NOT shared with ET).
    pub reader: R,
    /// Latest finalized hash provider (used as `at` block for snapshot lookup).
    pub latest_finalized_hash: LatestFinalizedHash,
    /// Per-epoch `BlsScheme` provider — used for local pre-submit
    /// crypto verification.
    pub scheme_provider: crate::outer::EpochSchemeProvider,
    pub stale_fallback: Arc<dyn StaleEpochFallback>,
    /// TxPool transport. Production impl wraps signer + pool + provider.
    pub sink: Arc<dyn SlasherTxSink>,
    /// WAL writer half. Producer (`handle`) enqueues
    /// `(victim || calldata)` payloads here after `verify_pre_submit`.
    pub wal_writer: wal_queue::Writer<E, Vec<u8>>,
    /// WAL reader half. Consumer (`run_consumer`) dequeues + submits + acks.
    pub wal_reader: wal_queue::Reader<E, Vec<u8>>,
}

/// **** Actor не лучшее название, очень просто в них потеряться если их несколько п о проекту
pub struct Actor<E, R>
where
    E: Clock + Metrics + Spawner + Storage + Send + 'static,
    R: StakingStateRead + Send + Sync + 'static,
{
    context: ContextCell<E>,
    mailbox_rx: mpsc::UnboundedReceiver<Message>,
    staking_address: Address,
    chain_id: u64,
    reader: R,
    latest_finalized_hash: LatestFinalizedHash,
    /// Per-epoch scheme used for local pre-submit verification.
    scheme_provider: crate::outer::EpochSchemeProvider,
    stale_fallback: Arc<dyn StaleEpochFallback>,
    sink: Arc<dyn SlasherTxSink>,
    wal_writer: wal_queue::Writer<E, Vec<u8>>,
    /// WAL reader; consumer side. `Option` so it can be moved into the
    /// consumer task on `start()`.
    wal_reader: Option<wal_queue::Reader<E, Vec<u8>>>,
    /// Dedup: victim addresses where an attempt has been observed on-chain
    /// this session. Producer checks before enqueue; consumer populates after
    /// `Mined`/`Reverted` outcome.
    submitted_this_session: Arc<TokioMutex<HashSet<Address>>>,
}

impl<E, R> Actor<E, R>
where
    E: Clock + Metrics + Spawner + Storage + Send + Sync + 'static,
    R: StakingStateRead + Send + Sync + 'static,
{
    pub fn init(context: E, cfg: Config<R, E>) -> (Self, Mailbox) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mailbox = Mailbox::new(tx);

        let actor = Self {
            context: ContextCell::new(context),
            mailbox_rx: rx,
            staking_address: cfg.staking_address,
            chain_id: cfg.chain_id,
            reader: cfg.reader,
            latest_finalized_hash: cfg.latest_finalized_hash,
            scheme_provider: cfg.scheme_provider,
            stale_fallback: cfg.stale_fallback,
            sink: cfg.sink,
            wal_writer: cfg.wal_writer,
            wal_reader: Some(cfg.wal_reader),
            submitted_this_session: Arc::new(TokioMutex::new(HashSet::new())),
        };
        (actor, mailbox)
    }

    pub fn start(mut self) -> Handle<()> {
        // Spawn the consumer first so the WAL has a reader ready before any
        // producer enqueues fire. The consumer's `Handle` is detached: when
        // the producer exits (mailbox closed) it drops the writer, the
        // consumer's `recv()` then returns `None` after draining, and the
        // consumer task exits naturally.
        let reader = self
            .wal_reader
            .take()
            .expect("wal_reader present at start (Actor::init seeds Some)");
        let sink = self.sink.clone();
        let staking_address = self.staking_address;
        let submitted = self.submitted_this_session.clone();
        let consumer_ctx = self.context.with_label("slasher_consumer");
        let _consumer_handle = consumer_ctx.spawn(move |_| async move {
            run_consumer(reader, sink, staking_address, submitted).await;
        });

        spawn_cell!(self.context, self.run_producer().await)
    }

    async fn run_producer(mut self) {
        info!("slasher producer starting");
        // Transient-error retry buffer: a `handle` failure on a TRANSIENT cause
        // (no finalized hash yet at startup, RPC Err, scheme/committee not yet
        // registered for the evidence epoch) must NOT lose the Activity —
        // simplex reports a conflict exactly once and there is no replay path.
        // Re-attempt with a short backoff before pulling the next mailbox item,
        // up to a bound; only a PERMANENT failure (malformed/variant-mismatch
        // evidence, BiMap divergence) is dropped.
        let mut retry: Option<(Message, u32)> = None;
        loop {
            if let Some((activity, attempts)) = retry.take() {
                self.context.sleep(SLASHER_RETRY_BACKOFF).await;
                match self.handle(activity.clone()).await {
                    Ok(()) => {}
                    Err(HandleError::Permanent(e)) => {
                        warn!(?e, "slasher producer handle failed permanently; dropping evidence");
                    }
                    Err(HandleError::Transient(e)) => {
                        if attempts + 1 >= SLASHER_MAX_RETRIES {
                            error!(?e, attempts = attempts + 1,
                                "slasher producer exhausted retries; dropping slashable evidence");
                            metrics::counter!("slasher_evidence_dropped_total").increment(1);
                        } else {
                            debug!(?e, attempts = attempts + 1,
                                "slasher producer transient failure; will retry");
                            retry = Some((activity, attempts + 1));
                        }
                    }
                }
                continue;
            }
            let Some(activity) = self.mailbox_rx.recv().await else {
                break;
            };
            match self.handle(activity.clone()).await {
                Ok(()) => {}
                Err(HandleError::Permanent(e)) => {
                    warn!(?e, "slasher producer handle failed permanently; dropping evidence");
                }
                Err(HandleError::Transient(e)) => {
                    debug!(?e, "slasher producer transient failure; will retry");
                    retry = Some((activity, 0));
                }
            }
        }
        info!("slasher producer exiting");
    }

    #[instrument(skip_all, fields(kind, epoch, victim))]
    async fn handle(&mut self, activity: Message) -> Result<(), HandleError> {
        let Some(kind) = SlashKind::from_activity(&activity) else {
            return Ok(()); // not slashable — silently drop
        };
        let epoch = activity.epoch().get();
        tracing::Span::current().record("kind", tracing::field::debug(kind));
        tracing::Span::current().record("epoch", epoch);

        // No finalized hash yet (startup) is transient — the next finalization
        // supplies one and the buffered Activity retries.
        let head = (self.latest_finalized_hash)()
            .ok_or_else(|| HandleError::transient("no latest finalized hash available"))?;
        let snap = match self.reader.epoch_committee_snapshot(epoch, head) {
            Ok(s) if !s.validators.is_empty() => s,
            Ok(_) => {
                // Empty on-chain committee → fall through to durable cache.
                // A cache read error is transient (storage hiccup); a genuine
                // miss (prune cursor past the evidence epoch) is permanent.
                self.stale_fallback
                    .get_by_epoch(epoch)
                    .await
                    .map_err(|e| HandleError::transient(format!("stale cache read failed: {e:?}")))?
                    .ok_or_else(|| {
                        HandleError::permanent(format!(
                            "epoch {epoch} evidence: empty on-chain committee AND \
                             not in cache (prune cursor advanced past evidence epoch); \
                             this evidence is unrecoverable"
                        ))
                    })?
            }
            // An on-chain read error (RPC / state lookup) is transient.
            Err(e) => return Err(HandleError::transient(format!("committee read failed: {e:?}"))),
        };

        // Build typed EpochCommittee + resolve victim via the BiMap.
        let committee = epoch_committee_from_snapshot(&snap)
            .map_err(|e| HandleError::permanent(format!("epoch_committee_from_snapshot failed: {e:?}")))?;

        // Local pre-submit crypto verify. The scheme provider keeps only the
        // recent epochs (pruned ~8 deep), but on-chain `_slashEquivocation`
        // accepts evidence for `undelegatePeriod + 8` epochs with no age check
        // (§14). For an evidence epoch whose scheme is pruned but whose
        // committee the stale fallback still holds, the pre-submit window must
        // match the committee stale-fallback window — so verify against a
        // verifier rebuilt from the recovered `committee.bimap`.
        //
        // The per-epoch DKG polynomial is NOT recoverable at slash time, so the
        // rebuilt combined verifier would have `beacon = None`; on a
        // beacon-active chain that WRONGLY rejects a seeded Notarize/Finalize
        // equivocation vote (CombinedScheme::verify_attestation's
        // `_ => combined.seed.is_none()` fallback arm rejects a present seed).
        // The on-chain evidence is over the attributable VOTE half only (the
        // threshold seed partial is non-attributable and dropped), so on a
        // rebuild we verify ONLY that vote half against a VoteScheme verifier —
        // correct, sufficient, and polynomial-free. When the full per-epoch
        // scheme is still registered, use it directly (its seeded arm verifies
        // the partial too, a strictly stronger check).
        let mut rng = rand_core::OsRng;
        match self.scheme_provider.scoped(ConsensusEpoch::new(epoch)) {
            Some(arc) => {
                verify_pre_submit(&activity, arc.as_ref(), &mut rng).map_err(|e| {
                    HandleError::permanent(format!("verify_pre_submit rejected evidence: {e:?}"))
                })?;
            }
            None => {
                debug!(
                    epoch,
                    "BlsScheme pruned for evidence epoch; \
                     verifying vote-half against verifier rebuilt from cached committee"
                );
                let vote_scheme = fluentbase_bls::VoteScheme::verifier(
                    &fluentbase_bls::fluent_namespace(self.chain_id),
                    committee.bimap.clone(),
                );
                verify_pre_submit_vote_only(&activity, &vote_scheme, &mut rng).map_err(|e| {
                    HandleError::permanent(format!(
                        "verify_pre_submit (vote-only) rejected evidence: {e:?}"
                    ))
                })?;
            }
        }

        let args: SlashCallArgs = match (kind, &activity) {
            (SlashKind::ConflictingNotarize, Activity::ConflictingNotarize(ev)) => {
                extract_from_conflicting_notarize(ev, &committee)
                    .map_err(|e| HandleError::permanent(format!("{e:?}")))?
            }
            (SlashKind::ConflictingFinalize, Activity::ConflictingFinalize(ev)) => {
                extract_from_conflicting_finalize(ev, &committee)
                    .map_err(|e| HandleError::permanent(format!("{e:?}")))?
            }
            (SlashKind::NullifyFinalize, Activity::NullifyFinalize(ev)) => {
                extract_from_nullify_finalize(ev, &committee)
                    .map_err(|e| HandleError::permanent(format!("{e:?}")))?
            }
            // `SlashKind::from_activity` already filtered to a slashable variant,
            // so this is unreachable today; degrade gracefully (log + skip) rather
            // than panic the accountability actor if a future variant desyncs
            // `from_activity` from this match.
            _ => {
                return Err(HandleError::permanent(format!(
                    "SlashKind/Activity variant mismatch ({kind:?}); skipping"
                )))
            }
        };

        let signer_idx = activity_signer_idx(&activity).ok_or_else(|| {
            HandleError::permanent("Activity variant carries no slashable signer; skipping")
        })?;
        let signer_peer = committee
            .bimap
            .get(signer_idx as usize)
            .ok_or_else(|| HandleError::permanent(format!("signer_idx {signer_idx} not in BiMap")))?;
        let victim = snap
            .validators
            .iter()
            .find(|v| &v.keys.peer_pubkey == signer_peer)
            .ok_or_else(|| {
                HandleError::permanent(format!(
                    "BiMap-resolved peer pubkey not in snapshot — \
                     contract / BiMap ordering divergence; signer_idx={signer_idx}"
                ))
            })?
            .address;
        tracing::Span::current().record("victim", tracing::field::display(victim));

        // Skip enqueue if a Mined/Reverted outcome already observed
        // this session (preserves the existing dedup behaviour). The
        // consumer populates `submitted_this_session` after a non-Failed
        // outcome.
        {
            let dedup = self.submitted_this_session.lock().await;
            if dedup.contains(&victim) {
                debug!(%victim, "already slashed this session; skipping enqueue");
                return Ok(());
            }
        }

        let calldata = encode_calldata(&args);
        let payload = encode_wal_payload(victim, &calldata);
        let pos = self
            .wal_writer
            .enqueue(payload)
            .await
            .map_err(|e| HandleError::transient(format!("WAL enqueue failed: {e:?}")))?;
        debug!(%victim, pos, "enqueued slash evidence to WAL");
        metrics::counter!("slasher_wal_enqueued_total", "kind" => kind_label(kind)).increment(1);
        Ok(())
    }
}

/// Consumer task: dequeues `(victim || calldata)` payloads from the WAL,
/// hands them to the sink, and acks based on the outcome.
async fn run_consumer<E>(
    mut reader: wal_queue::Reader<E, Vec<u8>>,
    sink: Arc<dyn SlasherTxSink>,
    staking_address: Address,
    submitted: Arc<TokioMutex<HashSet<Address>>>,
) where
    E: Clock + Metrics + Spawner + Storage + Send + 'static,
{
    info!("slasher consumer starting");
    loop {
        match reader.recv().await {
            Ok(None) => break, // writer dropped — drain complete
            Ok(Some((pos, payload))) => {
                let (victim, calldata) = match decode_wal_payload(&payload) {
                    Ok(v) => v,
                    Err(e) => {
                        error!(pos, ?e, "WAL payload decode failed; acking to drop");
                        if let Err(ack_err) = reader.ack(pos).await {
                            error!(pos, ?ack_err, "WAL ack of malformed entry failed");
                        }
                        continue;
                    }
                };
                let outcome = sink.submit(staking_address, Bytes::from(calldata)).await;
                match outcome {
                    SubmitOutcome::Mined { tx_hash } => {
                        info!(%victim, %tx_hash, "slash mined");
                        metrics::counter!("slasher_submitted_total").increment(1);
                        submitted.lock().await.insert(victim);
                        if let Err(e) = reader.ack(pos).await {
                            error!(pos, ?e, "WAL ack failed after Mined");
                        }
                    }
                    SubmitOutcome::AlreadySlashed => {
                        // Pre-flight sim confirmed the victim is already
                        // tombstoned — goal achieved, ack without a tx.
                        info!(%victim, "victim already tombstoned (pre-flight); acking");
                        metrics::counter!("slasher_already_slashed_total").increment(1);
                        submitted.lock().await.insert(victim);
                        if let Err(e) = reader.ack(pos).await {
                            error!(pos, ?e, "WAL ack failed after AlreadySlashed");
                        }
                    }
                    SubmitOutcome::Failed(msg) => {
                        // Do NOT ack — entry re-delivered only after a restart
                        // (no automatic in-session retry). A simulated revert
                        // here is a deterministic bug (calldata/EIP-2537
                        // encoding) — alert; retrying the same bytes won't help.
                        error!(%victim, %msg, "slash submission/simulation failed");
                        metrics::counter!("slasher_submit_failed_total").increment(1);
                    }
                }
            }
            Err(e) => {
                error!(?e, "WAL recv failed; consumer exiting");
                break;
            }
        }
    }
    info!("slasher consumer exiting");
}

/// Convenience constructor: initialize the WAL queue under the slasher's
/// own context label. Called from `outer.rs::build` (async); the returned
/// `(Writer, Reader)` is passed into [`Config`].
pub async fn init_wal_queue<E>(
    context: E,
    partition: String,
) -> Result<(wal_queue::Writer<E, Vec<u8>>, wal_queue::Reader<E, Vec<u8>>), eyre::Report>
where
    E: Clock
        + Metrics
        + Spawner
        + Storage
        + commonware_runtime::BufferPooler
        + Send
        + Sync
        + 'static,
{
    use commonware_runtime::buffer::paged::CacheRef;
    use commonware_storage::queue::Config as QueueConfig;
    use commonware_utils::{NZUsize, NZU16, NZU64};

    let page_cache = CacheRef::from_pooler(&context, NZU16!(4096), NZUsize!(64));
    let cfg = QueueConfig {
        partition,
        // One section per ~256 slash events; pruning is exact at section
        // granularity (slashing is rare so granularity is uncritical).
        items_per_section: NZU64!(256),
        compression: None,
        // Vec<u8> codec: open-ended length range + unit cfg for u8.
        codec_config: ((0..).into(), ()),
        page_cache,
        write_buffer: NZUsize!(1 << 16),
    };

    let (writer, reader) = wal_queue::init::<_, Vec<u8>>(context, cfg)
        .await
        .map_err(|e| eyre::eyre!("queue::shared::init failed: {e:?}"))?;
    Ok((writer, reader))
}

fn activity_signer_idx(activity: &Activity<BlsScheme, Digest>) -> Option<u32> {
    match activity {
        Activity::ConflictingNotarize(ev) => Some(ev.signer().get()),
        Activity::ConflictingFinalize(ev) => Some(ev.signer().get()),
        Activity::NullifyFinalize(ev) => Some(ev.signer().get()),
        // Non-slashable variant (filtered upstream by SlashKind::from_activity);
        // None lets the caller skip gracefully rather than panic.
        _ => None,
    }
}

fn kind_label(kind: SlashKind) -> &'static str {
    match kind {
        SlashKind::ConflictingNotarize => "conflicting_notarize",
        SlashKind::ConflictingFinalize => "conflicting_finalize",
        SlashKind::NullifyFinalize => "nullify_finalize",
    }
}

fn encode_calldata(args: &SlashCallArgs) -> Vec<u8> {
    let evidence = Bytes::from(args.evidence.clone());
    let pk = Bytes::from(args.pk_uncompressed.to_vec());
    let s1 = Bytes::from(args.sig1_uncompressed.to_vec());
    let s2 = Bytes::from(args.sig2_uncompressed.to_vec());
    match args.kind {
        SlashKind::ConflictingNotarize => slashEquivocationNotarizeCall {
            evidence,
            pkUncompressed: pk,
            sig1Uncompressed: s1,
            sig2Uncompressed: s2,
        }
        .abi_encode(),
        SlashKind::ConflictingFinalize => slashEquivocationFinalizeCall {
            evidence,
            pkUncompressed: pk,
            sig1Uncompressed: s1,
            sig2Uncompressed: s2,
        }
        .abi_encode(),
        SlashKind::NullifyFinalize => slashEquivocationNullifyFinalizeCall {
            evidence,
            pkUncompressed: pk,
            sig1Uncompressed: s1,
            sig2Uncompressed: s2,
        }
        .abi_encode(),
    }
}

/// WAL payload format: 20 bytes victim address || N bytes ABI-encoded calldata.
fn encode_wal_payload(victim: Address, calldata: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + calldata.len());
    out.extend_from_slice(victim.as_slice());
    out.extend_from_slice(calldata);
    out
}

fn decode_wal_payload(bytes: &[u8]) -> Result<(Address, Vec<u8>), eyre::Report> {
    if bytes.len() < 20 {
        return Err(eyre::eyre!(
            "WAL payload too short: {} bytes (need >= 20)",
            bytes.len()
        ));
    }
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&bytes[..20]);
    Ok((Address::from(addr), bytes[20..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_address(byte: u8) -> Address {
        let mut a = [0u8; 20];
        a[0] = byte;
        Address::from(a)
    }

    #[test]
    fn encode_calldata_dispatches_on_kind() {
        let args = SlashCallArgs {
            kind: SlashKind::ConflictingNotarize,
            evidence: vec![0xAA, 0xBB],
            pk_uncompressed: [0xCC; fluentbase_bls::PUBKEY_EIP2537_BYTES],
            sig1_uncompressed: [0xDD; fluentbase_bls::SIGNATURE_EIP2537_BYTES],
            sig2_uncompressed: [0xEE; fluentbase_bls::SIGNATURE_EIP2537_BYTES],
        };
        let calldata_notarize = encode_calldata(&args);

        let args_fin = SlashCallArgs {
            kind: SlashKind::ConflictingFinalize,
            ..args.clone()
        };
        let calldata_finalize = encode_calldata(&args_fin);

        let args_nf = SlashCallArgs {
            kind: SlashKind::NullifyFinalize,
            ..args
        };
        let calldata_nullify_fin = encode_calldata(&args_nf);

        // Each variant must produce a distinct selector (first 4 bytes).
        assert_ne!(&calldata_notarize[..4], &calldata_finalize[..4]);
        assert_ne!(&calldata_finalize[..4], &calldata_nullify_fin[..4]);
        assert_ne!(&calldata_notarize[..4], &calldata_nullify_fin[..4]);
    }

    #[test]
    fn wal_payload_roundtrip() {
        let victim = make_address(0x42);
        let calldata = vec![0xAA, 0xBB, 0xCC];
        let payload = encode_wal_payload(victim, &calldata);
        assert_eq!(payload.len(), 23);
        let (dec_victim, dec_calldata) = decode_wal_payload(&payload).unwrap();
        assert_eq!(dec_victim, victim);
        assert_eq!(dec_calldata, calldata);
    }

    #[test]
    fn wal_payload_decode_rejects_short() {
        let too_short = vec![0u8; 10];
        let err = decode_wal_payload(&too_short).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }
}
