//! Durable backing for the in-memory [`SeedStore`](crate::beacon::certify::SeedStore):
//! a `Round → σ` store plus the startup rehydration that refills the RAM map
//! from it.
//!
//! ## Why the durable write may be asynchronous when the RAM write may not
//!
//! `SeedStore::record` is ORDERING-CRITICAL (`crate::spec_exec`): it must stay
//! synchronous and precede the executor send, because the certify gate's
//! `false`-on-missing-seed verdict is cross-node deterministic only if every
//! honest node has recorded the round before its own `certify` scan reaches it.
//! That contract constrains *in-RAM visibility within this process*. Durability
//! is only ever read by a LATER process, after a restart, so it is free to lag.
//! The store therefore hands each fresh record to [`spawn_writer`] over a
//! non-blocking unbounded channel and returns immediately; all disk work happens
//! on the spawned task.
//!
//! ## Layout
//!
//! One `commonware_storage::ordinal::Ordinal<E, BlsSignature>`, indexed by the
//! packed round (see [`index_of`]). `Ordinal` writes `value ‖ crc32(value)` at
//! `index * RECORD_SIZE` and verifies that CRC on every read, so σ carries its
//! own integrity check with no hand-rolled tag and no hand-rolled codec.
//!
//! ## Why not `journal::segmented::fixed`
//!
//! It was the original pick and it is the wrong shape for this data. Two
//! measured reasons:
//!
//! 1. **Blast radius.** The fixed journal's replay stream marks a blob `done` at
//!    the first decode error and abandons the rest of it
//!    (`journal/segmented/fixed.rs` replay unfold), and the paged buffer under
//!    it checksums whole 4 KiB pages: a page that fails its CRC is dropped
//!    wholesale, and if it is the last page the blob is silently truncated to
//!    the last valid one (`runtime/.../paged/append.rs::read_last_valid_page`).
//!    Measured on this code before the move: 8 rounds appended into one section,
//!    one bit flipped in record #3, **0 of 8 survived the reopen and the replay
//!    stream yielded no error at all**. Section = epoch, so one bad byte cost a
//!    whole epoch of σ, silently. `Ordinal` drops exactly the one record whose
//!    CRC fails and keeps scanning (`ordinal/storage.rs` interval rebuild).
//! 2. **Lookup shape.** `Ordinal` gives O(1) point lookup plus `has` /
//!    `next_gap` / `ranges` — the "which rounds can I serve, which am I missing"
//!    API the seed transport step needs — where the journal only offers a scan.
//!
//! ## Retention
//!
//! Expressed in ROUNDS, not blobs. Epoch length is not a constant (production
//! epochs are 86 400 blocks, devnet epochs are hundreds), so "keep the last K
//! blobs" would under-retain on short epochs and over-retain on long ones.
//! [`SeedJournal::prune_to_window`] walks the interval map newest-first
//! accumulating range lengths and keeps everything from the EPOCH at which the
//! accumulation first reaches the target. Right after an epoch rolls the new
//! epoch holds one round, so the previous epoch stays until the window refills.

use std::num::{NonZeroU64, NonZeroUsize};

use commonware_consensus::types::{Epoch, Round, View};
use commonware_runtime::{BufferPooler, Clock, Handle, Metrics, Spawner, Storage};
use commonware_storage::ordinal::{Config as OrdinalConfig, Error as OrdinalError, Ordinal};
use commonware_utils::{NZUsize, NZU64};
use fluentbase_bls::BlsSignature;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{info, warn};

/// Bits of the packed store index reserved for `view`.
///
/// **Why a packing is needed at all:** `Ordinal` is indexed by a bare `u64`, and
/// the key here is a `Round { epoch, view }`. `view` is NOT globally increasing
/// in this repo — a separate consensus engine is spawned per epoch
/// (`crate::engine`, partition `consensus_epoch_{n}`) and each voter starts at
/// view 1 — so `view` alone would collide across epochs.
const VIEW_BITS: u32 = 32;

/// Mask for the view half of a packed index.
const VIEW_MASK: u64 = (1u64 << VIEW_BITS) - 1;

/// Blob granularity.
///
/// **INVARIANT: `ITEMS_PER_BLOB == 1 << VIEW_BITS`, so blob section == epoch and
/// a blob can never straddle an epoch boundary.** `Ordinal::prune(min)` removes
/// whole blobs (`section < min / items_per_blob`), so this is exactly what makes
/// `prune(oldest_kept_epoch << VIEW_BITS)` drop whole prior epochs and nothing
/// else — the same granularity the section-per-epoch journal had. Changing
/// either constant without the other silently changes what `prune` deletes.
const ITEMS_PER_BLOB: NonZeroU64 = NZU64!(4_294_967_296);
const _: () = assert!(ITEMS_PER_BLOB.get() == 1u64 << VIEW_BITS);

/// Write buffer for the append path. Deliberately far smaller than the
/// crate-wide [`crate::WRITE_BUFFER`] (1 MiB): this store writes 52 bytes per
/// round and syncs every drained batch, so a large buffer would only hold idle
/// memory per open blob.
const SEED_WRITE_BUFFER: NonZeroUsize = NZUsize!(4 * 1024);

/// Read buffer for the startup interval rebuild. One pass over the retained
/// window.
const SEED_REPLAY_BUFFER: NonZeroUsize = NZUsize!(64 * 1024);

/// Upper bound on records written before the writer task forces a sync. Bounds
/// the unsynced window during a catch-up burst; in steady state (1 round/s) a
/// batch is one record.
const MAX_BATCH: usize = 256;

/// Failures of the durable seed store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("seed store: {0}")]
    Store(#[from] OrdinalError),
    /// The round cannot be packed into the `u64` store index. Loud by
    /// construction: a silent wrap would file σ under a DIFFERENT round, and σ
    /// is an execution input (`prev_randao = keccak256(σ)` lands in `mix_hash`),
    /// so a σ served for the wrong round moves the state root. Losing the record
    /// is a store miss; mis-filing it is a fork.
    #[error(
        "round epoch={epoch} view={view} is not representable in the seed store index: \
         both halves must fit in {VIEW_BITS} bits"
    )]
    UnrepresentableRound { epoch: u64, view: u64 },
}

/// Pack a [`Round`] into the store's `u64` index: `epoch << 32 | view`.
///
/// Order-preserving with respect to `Round`'s own `Ord` (epoch first, then
/// view), which is what lets the newest-first walk in [`SeedJournal::window_start`]
/// and [`SeedJournal::replay_window`] read the interval map directly.
///
/// Refuses rather than wraps when either half exceeds 32 bits — see
/// [`Error::UnrepresentableRound`]. The bound is not tight in practice:
/// production `epochBlockInterval` is 86 400 blocks
/// (`staking-reader/src/epoch_transition.rs` `PROD_INTERVAL`), so an epoch is
/// ~86 400 views plus whatever nullified views it accumulates — five orders of
/// magnitude below 2^32. The on-chain interval is a `uint32`, so even the
/// largest settable epoch would need ~2^32 views (≈136 years at 1 s) to reach
/// the bound.
fn index_of(round: Round) -> Result<u64, Error> {
    let epoch = round.epoch().get();
    let view = round.view().get();
    if epoch > VIEW_MASK || view > VIEW_MASK {
        return Err(Error::UnrepresentableRound { epoch, view });
    }
    Ok((epoch << VIEW_BITS) | view)
}

/// Inverse of [`index_of`].
fn round_of(index: u64) -> Round {
    Round::new(Epoch::new(index >> VIEW_BITS), View::new(index & VIEW_MASK))
}

/// The durable half of the seed store. Exactly ONE instance per process: a second
/// handle over the same partition is a dual-writer, and one of them would prune a
/// blob the other still holds open.
pub struct SeedJournal<E: Storage + Metrics + Clock + BufferPooler> {
    store: Ordinal<E, BlsSignature>,
    /// Highest epoch written to so far, so pruning runs only when an epoch
    /// actually rolls (blob == epoch ⇒ `prune` is a no-op otherwise).
    newest_epoch: Option<u64>,
}

impl<E: Storage + Metrics + Clock + BufferPooler> SeedJournal<E> {
    pub async fn init(context: E, partition: String) -> Result<Self, Error> {
        let store = Ordinal::init(
            context,
            OrdinalConfig {
                partition,
                items_per_blob: ITEMS_PER_BLOB,
                write_buffer: SEED_WRITE_BUFFER,
                replay_buffer: SEED_REPLAY_BUFFER,
            },
        )
        .await?;
        let newest_epoch = store.last_index().map(|i| i >> VIEW_BITS);
        Ok(Self {
            store,
            newest_epoch,
        })
    }

    /// Write one entry. Returns `true` when it opened an epoch the store had not
    /// written before — the writer task's signal that pruning is worth a look.
    pub async fn append(&mut self, round: Round, signature: BlsSignature) -> Result<bool, Error> {
        let index = index_of(round)?;
        self.store.put(index, signature).await?;
        let epoch = round.epoch().get();
        let rolled = self.newest_epoch.is_none_or(|n| epoch > n);
        if rolled {
            self.newest_epoch = Some(epoch);
        }
        Ok(rolled)
    }

    /// One fsync per blob touched since the last call.
    pub async fn sync(&mut self) -> Result<(), Error> {
        self.store.sync().await?;
        Ok(())
    }

    /// Load the newest `retention` entries, oldest-first.
    ///
    /// A record whose CRC failed is not in the interval map at all — `Ordinal`
    /// excludes it during the startup rebuild and keeps scanning the blob — so it
    /// is simply absent here, costing ONE round rather than the rest of its
    /// epoch. That is the whole reason this store is an `Ordinal` and not a
    /// segmented journal (see the module docs).
    pub async fn replay_window(
        &self,
        retention: usize,
    ) -> Result<Vec<(Round, BlsSignature)>, Error> {
        if retention == 0 {
            return Ok(Vec::new());
        }
        // Walk the interval map newest-first, collecting at most `retention`
        // indices. Ranges — not indices — are what is materialised, and a
        // contiguous range is one unbroken run of rounds, so this is a handful
        // of entries even for a full epoch.
        let ranges: Vec<(u64, u64)> = self.store.ranges().collect();
        let mut indices: Vec<u64> = Vec::with_capacity(retention.min(4096));
        'walk: for &(start, end) in ranges.iter().rev() {
            let mut index = end;
            loop {
                indices.push(index);
                if indices.len() >= retention {
                    break 'walk;
                }
                if index == start {
                    break;
                }
                index -= 1;
            }
        }
        indices.reverse();

        let mut loaded = Vec::with_capacity(indices.len());
        let mut rejected = 0u64;
        for index in indices {
            match self.store.get(index).await {
                Ok(Some(signature)) => loaded.push((round_of(index), signature)),
                // The interval map said the index is present, so both arms mean
                // the record went bad between the rebuild and now. Skip it: a
                // missing σ is a store miss, which is the pre-durability
                // behaviour, and never a wrong σ.
                Ok(None) => rejected += 1,
                Err(e) => {
                    rejected += 1;
                    warn!(index, ?e, "seed store: skipping an unreadable record");
                }
            }
        }
        metrics::counter!("dpos_seed_journal_replayed_total").increment(loaded.len() as u64);
        metrics::counter!("dpos_seed_journal_replay_rejected_total").increment(rejected);
        Ok(loaded)
    }

    /// Drop every epoch older than the one at which a newest-first walk first
    /// accumulates `retention` records.
    pub async fn prune_to_window(&mut self, retention: u64) -> Result<(), Error> {
        let Some(start) = self.window_start(retention) else {
            return Ok(());
        };
        self.store.prune(start).await?;
        Ok(())
    }

    /// The lowest index that must be kept: the first index of the epoch holding
    /// the `retention`-th newest record, or `None` when the store is empty.
    ///
    /// A contiguous index range never straddles an epoch, because consecutive
    /// epochs are `1 << VIEW_BITS` apart and no epoch reaches that many views
    /// ([`index_of`] refuses one that would), so `start >> VIEW_BITS` is the
    /// range's epoch.
    fn window_start(&self, retention: u64) -> Option<u64> {
        let mut accumulated = 0u64;
        let mut oldest_epoch = None;
        for (start, end) in self.store.ranges().collect::<Vec<_>>().into_iter().rev() {
            accumulated = accumulated.saturating_add(end - start + 1);
            oldest_epoch = Some(start >> VIEW_BITS);
            if accumulated >= retention {
                break;
            }
        }
        oldest_epoch.map(|epoch| epoch << VIEW_BITS)
    }

    /// Epochs the store still holds records for, oldest-first. Used by the
    /// pruning tests; also the natural shape for any future "what can I serve"
    /// query.
    #[cfg(test)]
    fn retained_epochs(&self) -> Vec<u64> {
        let mut epochs: Vec<u64> = self.store.ranges().map(|(s, _)| s >> VIEW_BITS).collect();
        epochs.dedup();
        epochs
    }
}

/// Drive a [`SeedJournal`] from the store's record channel.
///
/// Each wakeup drains everything already queued, writes the batch, and issues
/// ONE sync for it. In steady state (1 round/s) that is one fsync per second of a
/// 52-byte write; under a catch-up burst the batching makes the fsync rate
/// self-limiting. The unsynced window is therefore bounded by one drain.
///
/// ## What the returned handle guarantees, and what it does not
///
/// `UnboundedReceiver::recv` yields every buffered item before it returns `None`,
/// so once the LAST [`SeedStore`](crate::beacon::certify::SeedStore) clone drops
/// (dropping the sender) this loop makes one final pass — write the remainder,
/// sync it — and only then exits. Awaiting the returned [`Handle`] therefore
/// waits for the tail to be ON DISK, and the node's graceful-shutdown path does
/// exactly that (`crates/node/src/dpos.rs`, `drain_shutdown_tasks`) after the
/// engine that owns the store is down.
///
/// The handle is NOT a supervision handle: its resolution means "the writer
/// finished its work", the opposite of the `supervised` vec's "something died,
/// bring the node down". Do not conflate the two.
///
/// What is still lost: a hard kill (SIGKILL, power cut) and a drain that exceeds
/// the shutdown timeout. Both degrade to store MISSES — the pre-4.1 behaviour
/// after every restart — never to a wrong σ.
#[must_use = "the returned handle must be awaited on shutdown or the tail is lost"]
pub fn spawn_writer<E>(
    context: E,
    mut journal: SeedJournal<E>,
    mut rx: UnboundedReceiver<(Round, BlsSignature)>,
    retention: u64,
) -> Handle<()>
where
    E: Storage + Metrics + Clock + Spawner + BufferPooler + Clone + Send + 'static,
{
    context.spawn(move |_| async move {
        while let Some(first) = rx.recv().await {
            let mut rolled = false;
            let mut appended = 0u64;
            let mut next = Some(first);
            while let Some((round, signature)) = next {
                match journal.append(round, signature).await {
                    Ok(opened_epoch) => {
                        rolled |= opened_epoch;
                        appended += 1;
                    }
                    Err(e) => warn!(?round, ?e, "seed store write failed; seed stays RAM-only"),
                }
                if appended as usize >= MAX_BATCH {
                    break;
                }
                next = rx.try_recv().ok();
            }
            metrics::counter!("dpos_seed_journal_appended_total").increment(appended);
            if let Err(e) = journal.sync().await {
                metrics::counter!("dpos_seed_journal_sync_failed_total").increment(1);
                // The shutdown drain re-syncs, but a sync that FAILS is not
                // rescued by waiting for it — this batch is at risk on ANY exit,
                // clean or not, until a later sync of the same blob succeeds.
                warn!(?e, "seed store sync failed; this batch is at risk on exit");
            }
            if rolled {
                if let Err(e) = journal.prune_to_window(retention).await {
                    warn!(
                        ?e,
                        "seed store prune failed; the window is oversized, not wrong"
                    );
                }
            }
        }
        info!("seed store writer stopped: the store's record channel closed");
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::certify::{SeedStore, SEED_RETENTION};
    use commonware_codec::FixedSize;
    use commonware_cryptography::bls12381::primitives::{group::Private, ops, variant::MinSig};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Blob as _, Runner as _};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// On-disk width of one `Ordinal` record: the value plus its CRC32.
    const RECORD_SIZE: u64 = <BlsSignature as FixedSize>::SIZE as u64 + 4;

    /// A distinct, valid signature per round. `BlsSignature` is a curve point, so
    /// it cannot be fabricated from arbitrary bytes — sign with a per-round key.
    fn sig_for(round: Round) -> BlsSignature {
        let mut rng = StdRng::seed_from_u64(round.epoch().get() << 32 | round.view().get());
        ops::sign_message::<MinSig>(&Private::random(&mut rng), b"ns", b"seed-journal-test")
    }

    fn round_at(view: u64) -> Round {
        Round::new(Epoch::new(0), View::new(view))
    }

    // The whole loop, end to end and through the real writer task:
    //   SeedStore::record  →  channel  →  spawn_writer  →  store + fsync
    //   →  restart  →  replay_window  →  SeedStore::with_persistence  →  lookup
    // This is the claim Band 4.1 exists to make, so it is asserted against the
    // production path rather than against the store API directly.
    #[test]
    fn a_recorded_seed_reaches_disk_through_the_writer_and_survives_a_restart() {
        deterministic::Runner::default().start(|ctx| async move {
            let rounds: Vec<Round> = (1..=8).map(round_at).collect();

            let journal = SeedJournal::init(ctx.with_label("boot1"), "seeds".into())
                .await
                .expect("open");
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let writer = spawn_writer(ctx.with_label("writer"), journal, rx, SEED_RETENTION as u64);

            let store = SeedStore::with_persistence(Vec::new(), tx);
            for r in &rounds {
                store.record(*r, sig_for(*r));
            }
            // Drop the store so the channel closes; the writer drains what is
            // queued and exits, which is also the shutdown path in production —
            // where the node likewise AWAITS this handle rather than sleeping.
            drop(store);
            writer.await.expect("writer exits cleanly");

            let reopened = SeedJournal::init(ctx.with_label("boot2"), "seeds".into())
                .await
                .expect("reopen");
            let rehydrated = reopened
                .replay_window(SEED_RETENTION)
                .await
                .expect("replay");
            assert_eq!(
                rehydrated.len(),
                rounds.len(),
                "every recorded round reached disk through the writer task"
            );

            let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
            let restarted = SeedStore::with_persistence(rehydrated, tx2);
            for r in &rounds {
                assert_eq!(
                    restarted.lookup(*r),
                    Some(sig_for(*r)),
                    "the restarted store returns the same sigma the first one recorded"
                );
            }
        });
    }

    /// The shutdown contract `crates/node/src/dpos.rs::drain_shutdown_tasks`
    /// leans on: records QUEUED BUT NOT YET WRITTEN when the last sender drops
    /// are still written and fsynced before the writer's handle resolves.
    ///
    /// Nothing sleeps here, deliberately — the `.await` of the handle IS the
    /// assertion, and it is the same await the node performs. Nothing between
    /// `spawn_writer` and `drop(store)` yields, so the writer task has not been
    /// polled even once by then: every record below is still sitting in the
    /// channel, i.e. this is the tail case and not a lucky steady state.
    ///
    /// The reopen also pins the SYNC, not just the write: the whole 32-record
    /// tail (32 × 52 B) fits inside `SEED_WRITE_BUFFER`, and the deterministic
    /// runtime's storage publishes a blob's bytes into the shared partition map
    /// only on `sync` (`runtime/src/storage/memory.rs`), so a written-but-
    /// unsynced tail would replay as zero entries here.
    #[test]
    fn dropping_the_last_sender_drains_and_syncs_the_tail_before_the_handle_resolves() {
        deterministic::Runner::default().start(|ctx| async move {
            let rounds: Vec<Round> = (1..=32).map(round_at).collect();

            let journal = SeedJournal::init(ctx.with_label("boot1"), "seeds".into())
                .await
                .expect("open");
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let writer = spawn_writer(ctx.with_label("writer"), journal, rx, SEED_RETENTION as u64);

            let store = SeedStore::with_persistence(Vec::new(), tx);
            for r in &rounds {
                store.record(*r, sig_for(*r));
            }
            drop(store);
            writer
                .await
                .expect("the writer exits cleanly once the channel closes");

            let reopened = SeedJournal::init(ctx.with_label("boot2"), "seeds".into())
                .await
                .expect("reopen");
            let rehydrated = reopened
                .replay_window(SEED_RETENTION)
                .await
                .expect("replay");
            assert_eq!(
                rehydrated.len(),
                rounds.len(),
                "the whole queued tail was written AND synced before the handle resolved"
            );
            for (r, sig) in &rehydrated {
                assert_eq!(*sig, sig_for(*r), "each drained sigma is the one recorded");
            }
        });
    }

    #[test]
    fn record_survives_restart_and_returns_the_same_sigma() {
        deterministic::Runner::default().start(|ctx| async move {
            let rounds: Vec<Round> = (1..=5).map(round_at).collect();

            let mut journal = SeedJournal::init(ctx.with_label("boot1"), "seeds".into())
                .await
                .expect("open");
            for r in &rounds {
                journal.append(*r, sig_for(*r)).await.expect("append");
            }
            journal.sync().await.expect("sync");
            drop(journal);

            // Restart: a fresh handle over the SAME partition, as a new process
            // would open it. The label differs only because the deterministic
            // runtime refuses to register the same metric name twice within one
            // process; the storage underneath is the same.
            let reopened = SeedJournal::init(ctx.with_label("boot2"), "seeds".into())
                .await
                .expect("reopen");
            let rehydrated = reopened
                .replay_window(SEED_RETENTION)
                .await
                .expect("replay");
            assert_eq!(rehydrated.len(), rounds.len());

            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            let store = SeedStore::with_persistence(rehydrated, tx);
            for r in &rounds {
                assert_eq!(
                    store.lookup(*r),
                    Some(sig_for(*r)),
                    "the rehydrated store returns the same sigma it recorded"
                );
            }
        });
    }

    #[test]
    fn a_round_outside_the_persisted_window_misses_as_it_does_today() {
        deterministic::Runner::default().start(|ctx| async move {
            let mut journal = SeedJournal::init(ctx.clone(), "seeds".into())
                .await
                .expect("open");
            for v in 1..=3 {
                let r = round_at(v);
                journal.append(r, sig_for(r)).await.expect("append");
            }
            journal.sync().await.expect("sync");
            let rehydrated = journal.replay_window(SEED_RETENTION).await.expect("replay");

            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            let store = SeedStore::with_persistence(rehydrated, tx);
            assert_eq!(
                store.lookup(round_at(99)),
                None,
                "a round that was never persisted misses, exactly as with a RAM-only store"
            );
            assert_eq!(SeedStore::new().lookup(round_at(99)), None);
        });
    }

    #[test]
    fn the_rehydrated_window_is_capped_at_the_retention_bound() {
        deterministic::Runner::default().start(|ctx| async move {
            let mut journal = SeedJournal::init(ctx.clone(), "seeds".into())
                .await
                .expect("open");
            let over = SEED_RETENTION + 16;
            for v in 1..=over as u64 {
                let r = round_at(v);
                journal.append(r, sig_for(r)).await.expect("append");
            }
            journal.sync().await.expect("sync");

            let rehydrated = journal.replay_window(SEED_RETENTION).await.expect("replay");
            assert_eq!(rehydrated.len(), SEED_RETENTION);
            assert_eq!(
                rehydrated.first().map(|(r, _)| *r),
                Some(round_at(17)),
                "the OLDEST entries are the ones dropped"
            );
            assert_eq!(
                rehydrated.last().map(|(r, _)| *r),
                Some(round_at(over as u64))
            );
        });
    }

    #[test]
    fn prune_keeps_the_retention_window_across_an_epoch_roll() {
        deterministic::Runner::default().start(|ctx| async move {
            let mut journal = SeedJournal::init(ctx.clone(), "seeds".into())
                .await
                .expect("open");
            // Three short epochs of four rounds each.
            for epoch in 0..3u64 {
                for view in 0..4u64 {
                    let r = Round::new(Epoch::new(epoch), View::new(view));
                    journal.append(r, sig_for(r)).await.expect("append");
                }
            }
            journal.sync().await.expect("sync");

            // A window of 5 rounds needs the newest epoch (4 rounds) plus the one
            // before it; epoch 0 is droppable.
            journal.prune_to_window(5).await.expect("prune");
            assert_eq!(
                journal.retained_epochs(),
                vec![1, 2],
                "pruning is expressed in rounds and resolved against per-epoch counts"
            );

            // A window larger than everything held keeps everything.
            journal.prune_to_window(1_000).await.expect("prune");
            assert_eq!(journal.retained_epochs(), vec![1, 2]);
        });
    }

    // ---- the packed index -------------------------------------------------

    /// The reason the packing exists: views RESET per epoch (a fresh consensus
    /// engine per epoch, `crate::engine`), so view 1 of epoch N+1 must still sort
    /// after the last view of epoch N. Asserted at the arithmetic level and then
    /// end to end through the store's own ordering.
    #[test]
    fn the_packed_index_orders_an_epoch_boundary_the_way_round_does() {
        let last_of_n = Round::new(Epoch::new(3), View::new(VIEW_MASK));
        let first_of_next = Round::new(Epoch::new(4), View::new(1));
        assert!(
            last_of_n < first_of_next,
            "Round's own ordering, for reference"
        );
        assert!(
            index_of(last_of_n).unwrap() < index_of(first_of_next).unwrap(),
            "view 1 of epoch N+1 must sort after the LAST view of epoch N"
        );
        // Round-trip both halves.
        for r in [
            last_of_n,
            first_of_next,
            Round::new(Epoch::new(0), View::new(0)),
        ] {
            assert_eq!(round_of(index_of(r).unwrap()), r);
        }
        // A per-epoch view reset must not collide.
        assert_ne!(
            index_of(Round::new(Epoch::new(1), View::new(1))).unwrap(),
            index_of(Round::new(Epoch::new(2), View::new(1))).unwrap(),
        );
    }

    #[test]
    fn the_store_returns_an_epoch_boundary_in_round_order() {
        deterministic::Runner::default().start(|ctx| async move {
            let mut journal = SeedJournal::init(ctx.clone(), "seeds".into())
                .await
                .expect("open");
            let rounds = [
                Round::new(Epoch::new(3), View::new(7)),
                Round::new(Epoch::new(3), View::new(8)),
                Round::new(Epoch::new(4), View::new(1)),
                Round::new(Epoch::new(4), View::new(2)),
            ];
            for r in rounds {
                journal.append(r, sig_for(r)).await.expect("append");
            }
            journal.sync().await.expect("sync");

            let all = journal.replay_window(SEED_RETENTION).await.expect("replay");
            assert_eq!(
                all.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
                rounds.to_vec(),
                "oldest-first across the boundary, low views of the new epoch last"
            );

            // The retention walk is newest-first, so a window of 2 must be the
            // two rounds of the NEW epoch, not the two high views of the old one.
            let window = journal.replay_window(2).await.expect("replay");
            assert_eq!(
                window.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
                vec![rounds[2], rounds[3]]
            );
        });
    }

    /// A view that does not fit the packing must be REFUSED, never wrapped: a
    /// wrapped index files σ under a different round, and σ is an execution
    /// input.
    #[test]
    fn a_round_beyond_the_packing_bound_is_refused_not_wrapped() {
        let too_wide = Round::new(Epoch::new(0), View::new(1u64 << VIEW_BITS));
        assert!(
            matches!(index_of(too_wide), Err(Error::UnrepresentableRound { .. })),
            "a view of 2^32 must not silently become view 0 of epoch 1"
        );
        // The value it WOULD have wrapped onto is a real, different round.
        assert_eq!(
            index_of(Round::new(Epoch::new(1), View::new(0))).unwrap(),
            1u64 << VIEW_BITS,
            "the collision the refusal prevents"
        );
        assert!(matches!(
            index_of(Round::new(Epoch::new(1u64 << VIEW_BITS), View::new(0))),
            Err(Error::UnrepresentableRound { .. })
        ));

        // And the refusal reaches `append` rather than writing somewhere wrong.
        deterministic::Runner::default().start(|ctx| async move {
            let mut journal = SeedJournal::init(ctx.clone(), "seeds".into())
                .await
                .expect("open");
            let err = journal
                .append(too_wide, sig_for(round_at(1)))
                .await
                .expect_err("append must refuse the round");
            assert!(matches!(err, Error::UnrepresentableRound { .. }));
            journal.sync().await.expect("sync");
            assert!(
                journal
                    .replay_window(SEED_RETENTION)
                    .await
                    .expect("replay")
                    .is_empty(),
                "nothing was written under a wrapped index"
            );
        });
    }

    // ---- corruption blast radius ------------------------------------------

    /// Flip one bit inside the CRC32 trailer of the record for `round`, in the
    /// blob `Ordinal` stores it in. Targeting the CRC (not the value) isolates
    /// the check under test: the 48-byte signature still decodes as a curve
    /// point, so ONLY the per-record CRC can reject this record.
    ///
    /// Blob name is the section big-endian, section == epoch by the
    /// `ITEMS_PER_BLOB` invariant, and the record sits at `view * RECORD_SIZE`
    /// within it (`ordinal/storage.rs`).
    async fn corrupt_crc_of(ctx: &deterministic::Context, partition: &str, round: Round) {
        use commonware_runtime::Storage as _;
        let section = round.epoch().get();
        let (blob, _len) = ctx
            .open(partition, &section.to_be_bytes())
            .await
            .expect("open the blob directly");
        let crc_offset =
            round.view().get() * RECORD_SIZE + <BlsSignature as FixedSize>::SIZE as u64;
        let byte: u8 = blob
            .read_at(crc_offset, 1)
            .await
            .expect("read")
            .coalesce()
            .as_ref()[0]
            ^ 0x01;
        blob.write_at(crc_offset, vec![byte]).await.expect("write");
        blob.sync().await.expect("sync");
    }

    /// Lay down `views` rounds of epoch 0 into `partition`, synced.
    async fn seeded_epoch(ctx: &deterministic::Context, label: &str, partition: &str, views: u64) {
        let mut journal = SeedJournal::init(ctx.with_label(label), partition.into())
            .await
            .expect("open");
        for v in 0..views {
            let r = round_at(v);
            journal.append(r, sig_for(r)).await.expect("append");
        }
        journal.sync().await.expect("sync");
    }

    /// THE regression gate for the move off `journal::segmented::fixed`.
    ///
    /// A corrupt record must cost exactly ONE round. Under the segmented journal
    /// this same fixture lost all 8 (measured before the move: one flipped bit in
    /// record #3 left 0 of 8 readable, and the replay stream reported no error at
    /// all, because the paged buffer's 4 KiB page CRC failed and the blob was
    /// truncated back to the last valid page).
    ///
    /// The `control` half is the negative control: identical fixture, no
    /// corruption. It must yield all 8 — otherwise "7 survived" would be
    /// consistent with a fixture that simply cannot write 8.
    #[test]
    fn a_corrupt_record_costs_exactly_one_round_not_the_rest_of_the_epoch() {
        deterministic::Runner::default().start(|ctx| async move {
            const VIEWS: u64 = 8;
            const VICTIM: u64 = 3;

            // --- negative control: same fixture in its own partition, nothing
            // corrupted ---
            seeded_epoch(&ctx, "control1", "seeds_control", VIEWS).await;
            let control_rounds = {
                let reopened =
                    SeedJournal::init(ctx.with_label("control2"), "seeds_control".into())
                        .await
                        .expect("reopen");
                reopened
                    .replay_window(SEED_RETENTION)
                    .await
                    .expect("replay")
                    .into_iter()
                    .map(|(r, _)| r.view().get())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                control_rounds,
                (0..VIEWS).collect::<Vec<_>>(),
                "negative control: with no corruption the fixture yields every round, \
                 so the corrupted run below cannot pass by writing fewer"
            );

            // --- the real case, in its own partition ---
            seeded_epoch(&ctx, "corrupt1", "seeds_corrupt", VIEWS).await;
            corrupt_crc_of(&ctx, "seeds_corrupt", round_at(VICTIM)).await;

            let reopened = SeedJournal::init(ctx.with_label("corrupt2"), "seeds_corrupt".into())
                .await
                .expect("reopen");
            let survived: Vec<u64> = reopened
                .replay_window(SEED_RETENTION)
                .await
                .expect("replay")
                .into_iter()
                .map(|(r, _)| r.view().get())
                .collect();

            let expected: Vec<u64> = (0..VIEWS).filter(|v| *v != VICTIM).collect();
            assert_eq!(
                survived, expected,
                "one corrupt record costs exactly that round — every round AFTER it in \
                 the same epoch is still readable"
            );
            assert_eq!(
                survived.len() as u64,
                VIEWS - 1,
                "blast radius is 1 record, not the epoch"
            );
        });
    }

    /// The corruption is real, and the CRC is what catches it: the same fixture
    /// with the flip applied and then UNDONE reads back clean, so the missing
    /// round above is caused by the flipped bit and nothing else.
    #[test]
    fn undoing_the_flip_restores_the_round() {
        deterministic::Runner::default().start(|ctx| async move {
            seeded_epoch(&ctx, "b1", "seeds", 8).await;
            corrupt_crc_of(&ctx, "seeds", round_at(3)).await;
            corrupt_crc_of(&ctx, "seeds", round_at(3)).await; // flip back

            let reopened = SeedJournal::init(ctx.with_label("b2"), "seeds".into())
                .await
                .expect("reopen");
            let survived: Vec<u64> = reopened
                .replay_window(SEED_RETENTION)
                .await
                .expect("replay")
                .into_iter()
                .map(|(r, _)| r.view().get())
                .collect();
            assert_eq!(survived, (0..8).collect::<Vec<_>>());
        });
    }
}
