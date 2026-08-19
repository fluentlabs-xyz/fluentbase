//! Durable backing for [`BeaconKeys`](crate::beacon::keys::BeaconKeys): an
//! `epoch → PK_epoch` store plus the startup rehydration that refills the RAM
//! map from it.
//!
//! ## Only attested keys are persisted
//!
//! [`KeySource`] has three tiers and exactly one reaches the disk.
//!
//! - [`KeySource::ObservedOutcome`] — read out of a finalized, quorum-certified
//!   block. Agreed chain data, and the expensive tier to re-obtain after a
//!   restart: a node with no ceremony material of its own has to walk boundary
//!   blocks or fetch one from a peer. **Persisted.**
//! - [`KeySource::LocalDkg`] — this node's own reconstruction from the shares it
//!   received. It CAN diverge from what the network agreed (soak 2026-07-14: a
//!   stale local write beat the network's and poisoned the epoch), and today a
//!   restart CLEARS it — accidental protection, but real. Persisting it would let
//!   a wrong value outlive the restart that currently removes it, and it costs
//!   nothing to lose: the ceremony material it derives from is already on disk
//!   under `<datadir>/beacon/` and regenerates it at startup. **Never persisted.**
//! - [`KeySource::Carried`] — the memo saying an epoch that did not re-mint uses
//!   an earlier epoch's key. A cache of a derivation, not a fact; losing it costs
//!   one local walk over blocks already on disk. **Not persisted.**
//!
//! ## Why the durable write may lag the RAM write
//!
//! Lifted verbatim from [`crate::beacon::seed_journal`], because the argument is
//! the same one: the RAM write is what in-process readers see and must stay
//! synchronous, while durability is only ever read by a LATER process, after a
//! restart, so it is free to lag. Records go out over a non-blocking unbounded
//! channel and all disk work happens on the spawned writer.
//!
//! ## Retention: none
//!
//! Deliberate, and the one place this store differs from the seed journal, which
//! keeps a rolling window of rounds.
//!
//! An `ObservedOutcome` entry is written once per COMMITTEE CHANGE, not once per
//! epoch — and the store is keyed by the epoch that MINTED the key, so on a
//! committee that has been stable for a long time the entry worth having is the
//! OLDEST one. Any window measured in epochs therefore drops the valuable record
//! first and keeps nothing but recent noise, which is a journal that fails at
//! exactly the case it exists for.
//!
//! The arithmetic that makes "just keep them" the right answer rather than a
//! shrug: a record is a 96-byte G2 point + a 1-byte source tag + `Ordinal`'s
//! 4-byte CRC ≈ 101 B. A pathological chain that changed committee EVERY epoch
//! at a day-long epoch writes ~37 KB/year. A realistic one writes a few hundred
//! bytes a year.

use crate::beacon::{
    keys::{BeaconKeys, KeySource},
    seed::GroupPublic,
};
use bytes::{Buf, BufMut};
use commonware_codec::{Error as CodecError, FixedSize, Read, ReadExt as _, Write};
use commonware_runtime::{BufferPooler, Clock, Handle, Metrics, Spawner, Storage};
use commonware_storage::ordinal::{Config as OrdinalConfig, Error as OrdinalError, Ordinal};
use commonware_utils::{NZUsize, NZU64};
use std::num::{NonZeroU64, NonZeroUsize};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::warn;

/// Blob granularity. Entries are sparse (one per committee change) and the index
/// IS the epoch, so a blob spans a wide band of epochs and most are empty — which
/// costs nothing, `Ordinal` only materialises what was written.
const ITEMS_PER_BLOB: NonZeroU64 = NZU64!(65_536);

/// Write buffer. One ~101-byte record per committee change, so a large buffer
/// would only hold idle memory per open blob.
const KEY_WRITE_BUFFER: NonZeroUsize = NZUsize!(4 * 1024);

/// Read buffer for the startup interval rebuild.
const KEY_REPLAY_BUFFER: NonZeroUsize = NZUsize!(64 * 1024);

/// Upper bound on records written before the writer task forces a sync.
const MAX_BATCH: usize = 64;

/// Failures of the durable key store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("key store: {0}")]
    Store(#[from] OrdinalError),
}

/// One on-disk record: the key and the provenance it was written under.
///
/// The tag is written even though only one tier is persisted today. Without it,
/// admitting a second tier later would be an on-disk FORMAT change on a store
/// that already has records in it; with it, a restored entry can never acquire a
/// provenance it did not have — which is the property
/// [`BeaconKeys::attested`](crate::beacon::keys::BeaconKeys::attested) rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRecord {
    pub pk: GroupPublic,
    pub source: KeySource,
}

impl KeyRecord {
    const LOCAL_DKG: u8 = 0;
    const CARRIED: u8 = 1;
    const OBSERVED_OUTCOME: u8 = 2;

    fn tag(source: KeySource) -> u8 {
        match source {
            KeySource::LocalDkg => Self::LOCAL_DKG,
            KeySource::Carried => Self::CARRIED,
            KeySource::ObservedOutcome => Self::OBSERVED_OUTCOME,
        }
    }
}

impl FixedSize for KeyRecord {
    const SIZE: usize = GroupPublic::SIZE + std::mem::size_of::<u8>();
}

impl Write for KeyRecord {
    fn write(&self, buf: &mut impl BufMut) {
        self.pk.write(buf);
        Self::tag(self.source).write(buf);
    }
}

impl Read for KeyRecord {
    type Cfg = ();
    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let pk = GroupPublic::read(buf)?;
        let source = match u8::read(buf)? {
            Self::LOCAL_DKG => KeySource::LocalDkg,
            Self::CARRIED => KeySource::Carried,
            Self::OBSERVED_OUTCOME => KeySource::ObservedOutcome,
            _ => return Err(CodecError::Invalid("KeyRecord", "unknown key source tag")),
        };
        Ok(Self { pk, source })
    }
}

/// The durable half of the key store. Exactly ONE instance per process: a second
/// handle over the same partition is a dual-writer.
pub struct KeyJournal<E: Storage + Metrics + Clock + BufferPooler> {
    store: Ordinal<E, KeyRecord>,
}

impl<E: Storage + Metrics + Clock + BufferPooler> KeyJournal<E> {
    pub async fn init(context: E, partition: String) -> Result<Self, Error> {
        let store = Ordinal::init(
            context,
            OrdinalConfig {
                partition,
                items_per_blob: ITEMS_PER_BLOB,
                write_buffer: KEY_WRITE_BUFFER,
                replay_buffer: KEY_REPLAY_BUFFER,
            },
        )
        .await?;
        Ok(Self { store })
    }

    /// Write one entry, or decline it. `false` = deliberately not persisted.
    ///
    /// The filter lives HERE rather than at the call site so it cannot be
    /// bypassed by a future producer: what may become durable is a property of
    /// this store, not of whoever happens to be writing.
    pub async fn append(
        &mut self,
        epoch: u64,
        pk: GroupPublic,
        source: KeySource,
    ) -> Result<bool, Error> {
        if source != KeySource::ObservedOutcome {
            return Ok(false);
        }
        self.store.put(epoch, KeyRecord { pk, source }).await?;
        Ok(true)
    }

    /// One fsync per blob touched since the last call.
    pub async fn sync(&mut self) -> Result<(), Error> {
        self.store.sync().await?;
        Ok(())
    }

    /// Every retained entry, oldest-first, for the startup refill.
    ///
    /// A record whose CRC failed is not in the interval map at all — `Ordinal`
    /// excludes it during the startup rebuild and keeps scanning — so it is
    /// simply absent here, costing ONE epoch's key rather than the rest of the
    /// blob. That is why this store is an `Ordinal`.
    pub async fn replay(&self) -> Result<Vec<(u64, KeyRecord)>, Error> {
        let ranges: Vec<(u64, u64)> = self.store.ranges().collect();
        let mut loaded = Vec::new();
        let mut rejected = 0u64;
        for (start, end) in ranges {
            for index in start..=end {
                match self.store.get(index).await {
                    Ok(Some(record)) => loaded.push((index, record)),
                    // The interval map said the index is present, so both arms
                    // mean the record went bad between the rebuild and now. Skip
                    // it: a missing key is a store miss, which is the
                    // pre-durability behaviour, and never a WRONG key.
                    Ok(None) => rejected += 1,
                    Err(e) => {
                        rejected += 1;
                        warn!(index, ?e, "key store: skipping an unreadable record");
                    }
                }
            }
        }
        metrics::counter!("dpos_key_journal_replayed_total").increment(loaded.len() as u64);
        metrics::counter!("dpos_key_journal_replay_rejected_total").increment(rejected);
        Ok(loaded)
    }
}

/// Open the durable key store and join it to a RAM map, or hand back a RAM-only
/// store when no partition is configured.
///
/// An EMPTY partition means "no journal" — the pre-durability behaviour, which is
/// what tests and any in-process run get.
///
/// `writer_context` MUST be a SIBLING of `journal_context`, never a clone: the
/// deterministic runtime panics on a duplicate metric registered under the same
/// label, and both halves register store metrics.
pub async fn open<E>(
    journal_context: E,
    writer_context: E,
    partition: &str,
) -> eyre::Result<(BeaconKeys, Option<Handle<()>>)>
where
    E: Storage + Metrics + Clock + Spawner + BufferPooler + Clone + Send + 'static,
{
    if partition.is_empty() {
        return Ok((BeaconKeys::new(), None));
    }
    let journal = KeyJournal::init(journal_context, partition.to_string())
        .await
        .map_err(|e| eyre::eyre!("opening the durable key store: {e}"))?;
    let rehydrated: Vec<(u64, GroupPublic, KeySource)> = journal
        .replay()
        .await
        .map_err(|e| eyre::eyre!("replaying the durable key store: {e}"))?
        .into_iter()
        .map(|(epoch, record)| (epoch, record.pk, record.source))
        .collect();
    tracing::info!(
        entries = rehydrated.len(),
        "rehydrated the beacon-key store from disk"
    );
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let writer = spawn_writer(writer_context, journal, rx);
    Ok((BeaconKeys::with_persistence(rehydrated, tx), Some(writer)))
}

/// Drain `rx` onto the journal. The RAM store hands records over and returns
/// immediately; every disk touch happens here.
pub fn spawn_writer<E>(
    context: E,
    mut journal: KeyJournal<E>,
    mut rx: UnboundedReceiver<(u64, GroupPublic, KeySource)>,
) -> Handle<()>
where
    E: Storage + Metrics + Clock + Spawner + BufferPooler + Clone + Send + 'static,
{
    context.spawn(move |_| async move {
        while let Some(first) = rx.recv().await {
            let mut appended = 0u64;
            let mut next = Some(first);
            while let Some((epoch, pk, source)) = next {
                match journal.append(epoch, pk, source).await {
                    Ok(true) => appended += 1,
                    // Declined by the source filter — the common case for
                    // `LocalDkg` and `Carried`, and not a failure.
                    Ok(false) => {}
                    Err(e) => warn!(epoch, ?e, "key store write failed; key stays RAM-only"),
                }
                if appended as usize >= MAX_BATCH {
                    break;
                }
                next = rx.try_recv().ok();
            }
            if appended == 0 {
                continue;
            }
            metrics::counter!("dpos_key_journal_appended_total").increment(appended);
            if let Err(e) = journal.sync().await {
                metrics::counter!("dpos_key_journal_sync_failed_total").increment(1);
                warn!(
                    ?e,
                    "key store sync failed; the unsynced tail is lost on a hard kill"
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::{DecodeExt as _, Encode as _};
    use commonware_cryptography::bls12381::{
        dkg::deal_anonymous,
        primitives::{sharing::Sharing, variant::MinSig},
    };
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{test_rng, N3f1, NZU32};

    fn key(seed: u64) -> GroupPublic {
        let mut rng = test_rng();
        for _ in 0..seed {
            let _: Sharing<MinSig> =
                deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4)).0;
        }
        let (sharing, _) = deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        *sharing.public()
    }

    /// The record carries its provenance, and the codec is what makes that true
    /// on disk. Dropping the tag would make every restored entry take whatever
    /// default the reader picked — and the two defaults are both wrong (all
    /// `Carried` disables the divergence guard, all `ObservedOutcome` lies to
    /// `attested`).
    ///
    /// Reds if the tag leaves the on-disk shape.
    #[test]
    fn a_record_round_trips_with_its_source() {
        for source in [
            KeySource::LocalDkg,
            KeySource::Carried,
            KeySource::ObservedOutcome,
        ] {
            let record = KeyRecord { pk: key(1), source };
            let back = KeyRecord::decode(record.encode()).expect("round trip");
            assert_eq!(back, record, "{source:?} must survive the codec");
        }
    }

    /// The phase's one load-bearing rule: only an ATTESTED key reaches the disk.
    ///
    /// `LocalDkg` can diverge from the chain and a restart currently clears it —
    /// persisting it would let a wrong value outlive the restart that removes it.
    /// `Carried` is a cache of a derivation, re-earned by one local walk. Neither
    /// belongs in a store whose whole purpose is to survive.
    ///
    /// Reds if the filter is removed or widened.
    #[test]
    fn only_an_attested_key_reaches_the_disk() {
        deterministic::Runner::default().start(|ctx| async move {
            let mut journal = KeyJournal::init(ctx, "keys".into()).await.expect("init");

            assert!(
                !journal
                    .append(7, key(1), KeySource::LocalDkg)
                    .await
                    .expect("append"),
                "a local reconstruction must never become durable"
            );
            assert!(
                !journal
                    .append(8, key(2), KeySource::Carried)
                    .await
                    .expect("append"),
                "a carry memo is a cache, not a fact"
            );
            assert!(
                journal
                    .append(9, key(3), KeySource::ObservedOutcome)
                    .await
                    .expect("append"),
                "and the attested one does"
            );
            journal.sync().await.expect("sync");

            let on_disk = journal.replay().await.expect("replay");
            assert_eq!(
                on_disk.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
                vec![9],
                "epochs 7 and 8 must not be on disk at all"
            );
        });
    }

    /// The point of the store: what was written survives a process boundary, and
    /// comes back with the provenance it was written under rather than one the
    /// reader guessed.
    #[test]
    fn an_attested_key_survives_a_restart_with_its_source() {
        deterministic::Runner::default().start(|ctx| async move {
            let expected = key(4);
            {
                let mut journal = KeyJournal::init(ctx.with_label("boot1"), "keys".into())
                    .await
                    .expect("init");
                journal
                    .append(12, expected, KeySource::ObservedOutcome)
                    .await
                    .expect("append");
                journal.sync().await.expect("sync");
            }

            let reopened = KeyJournal::init(ctx.with_label("boot2"), "keys".into())
                .await
                .expect("re-init");
            let loaded = reopened.replay().await.expect("replay");

            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].0, 12, "under the epoch it was written for");
            assert_eq!(loaded[0].1.pk, expected);
            assert_eq!(
                loaded[0].1.source,
                KeySource::ObservedOutcome,
                "provenance is restored, never defaulted"
            );
        });
    }
}
