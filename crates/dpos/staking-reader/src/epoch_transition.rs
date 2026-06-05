//! Finality-gated epoch-boundary orchestrator (epoch_transition).
//!
//! Injection-style library: every collaborator is a constructor param, so
//! this compiles and unit-tests today without the consensus / p2p / node
//! layers (only their *instances* — the live finalized stream, the real
//! `Oracle`, the node wiring — are deferred).
//!
//! Design invariants:
//! - finality-gated apply;
//! - write-once `track` (no re-track of a covered epoch; no reorg handling
//!   — finalized ⇒ irreversible);
//! - the frozen-committee snapshot is what is persisted;
//! - committee-size pre-check (typed error, not a deep commonware panic);
//! - cold-start reads the *current* finalized committee once (no point
//!   taking an outdated state);
//! - retention mirrors the contract's own `_pruneStaleCommittees`
//!   (`undelegatePeriod + EPOCH_COMMITTEE_RETENTION_MARGIN`).
//!
//! Retry / outcome invariants:
//! - `last_tracked_epoch` advances only after `boundary_tx.try_send`
//!   succeeds — a `Full` channel leaves the epoch un-tracked so the next
//!   finalized block retries.
//! - `on_finalized` returns a [`TransitionOutcome`] so the caller's
//!   error counter resets only on `EpochAdvanced(_)`, not on intra-epoch
//!   no-ops.

use alloy_primitives::B256;
use commonware_runtime::{BufferPooler, Metrics, Storage};
use commonware_utils::ordered::Set;
use core::future::Future;
use fluentbase_bls::PeerPubkey;

use crate::{
    cache::ValidatorSetCache,
    error::ReadError,
    reader::{
        check_committee_size, epoch_of_block, StakingStateRead, EPOCH_COMMITTEE_RETENTION_MARGIN,
    },
};

/// Outcome of [`EpochTransition::on_finalized`] — distinguishes an
/// intra-epoch no-op from an actual epoch advance. The dpos.rs
/// boundary-hook closure uses this to decide whether to reset its
/// consecutive-error counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// This block was an intra-epoch re-delivery of an already-tracked
    /// epoch, a still-empty missed-commit epoch, or a retry stalled
    /// on a full bridge channel. No epoch state was advanced.
    Intra,
    /// This block advanced `last_tracked_epoch` to the given value;
    /// the boundary trigger has been delivered to the consensus bridge.
    EpochAdvanced(u64),
}

/// Where the assembled peer set is delivered. p2p-agnostic on purpose:
/// `staking-reader` does not depend on `commonware-p2p`. The real adapter
/// `impl PeerSetSink for commonware_p2p::Manager<PublicKey = PeerPubkey>`
/// (a one-liner `Manager::track(self, epoch, set).await`) is written at the
/// `Oracle`-handle owner (the node wiring), where the `oracle.track` call
/// site lives. Style mirrors commonware's own traits (`-> impl Future + Send`,
/// not `async fn`, to stay clean under `-D warnings`).
pub trait PeerSetSink {
    fn track(&mut self, epoch: u64, peers: Set<PeerPubkey>) -> impl Future<Output = ()> + Send;
}

/// Drives finality-gated epoch boundaries: detect → frozen-committee
/// snapshot → size-check → persist (final) → `track` once → prune to the
/// contract's retention window.
///
/// Cache is held behind `Arc<tokio::sync::Mutex<_>>` so the slasher
/// can take a read lock from a separate task to fall back to historical
/// committees when the on-chain prune cursor has advanced past evidence
/// epoch (`get_by_epoch`). Only ET writes; the slasher only reads.
pub struct EpochTransition<R, S, E: Storage + Metrics + BufferPooler> {
    reader: R,
    cache: std::sync::Arc<tokio::sync::Mutex<ValidatorSetCache<E>>>,
    sink: S,
    /// commonware `max_peer_set_size` (injected by the node; committee-size guard input).
    max_peer_set_size: usize,
    /// Write-once guard: the epoch already fed to `track`.
    last_tracked_epoch: Option<u64>,
    /// Optional boundary trigger for 04's `OuterEngine::boundary_sender`. When
    /// `Some`, every successful (non-skipped) epoch boundary fires
    /// `(epoch, snapshot)` exactly once. `try_send` is used (lossy) — if 04's
    /// receiver is closed, the trigger is silently dropped (04 has already
    /// shut down).
    boundary_tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
    /// `epochBlockInterval` frozen on the first finalized block. The consensus
    /// `FixedEpocher` is frozen at startup, so this MUST be treated as fixed
    /// after genesis — honoring a live governance change here would diverge the
    /// two epoch authorities. A later on-chain change is logged and ignored.
    /// (Correct boundary-synced live re-interval is a separate, deferred task.)
    frozen_interval: Option<u32>,
    /// `dposActivationBlock` frozen on the first finalized block — origin for
    /// the relative epoch numbering (consensus `OriginEpocher` is frozen at
    /// startup, so this is treated as fixed identically to the interval).
    frozen_activation: Option<u64>,
}

impl<R, S, E> EpochTransition<R, S, E>
where
    R: StakingStateRead,
    S: PeerSetSink,
    E: Storage + Metrics + BufferPooler,
{
    pub fn new(
        reader: R,
        cache: std::sync::Arc<tokio::sync::Mutex<ValidatorSetCache<E>>>,
        sink: S,
        max_peer_set_size: usize,
        boundary_tx: Option<tokio::sync::mpsc::Sender<(u64, crate::reader::ValidatorSetSnapshot)>>,
    ) -> Self {
        Self {
            reader,
            cache,
            sink,
            max_peer_set_size,
            last_tracked_epoch: None,
            boundary_tx,
            frozen_interval: None,
            frozen_activation: None,
        }
    }

    /// Apply one **finalized** block `B` (delivered sequentially via
    /// commonware `Reporter Update::Block` + ack).
    ///
    /// Idempotent per epoch (write-once `track`): a re-delivery of the
    /// same epoch is a no-op, never a re-`track` (commonware would silently
    /// drop it anyway). Persist, track and prune are all individually
    /// idempotent (`prunable::Archive::put` skips duplicate indices;
    /// `sink.track` no-ops on a re-track), so a retry path stalled on
    /// a full bridge channel re-executes the upstream side effects safely.
    ///
    /// Returns [`TransitionOutcome`]:
    /// - `Intra` — intra-epoch re-delivery, missed-commit epoch, or a
    ///   retry path where `boundary_tx.try_send` failed; epoch state is
    ///   NOT advanced.
    /// - `EpochAdvanced(epoch)` — the bridge trigger was delivered and
    ///   `last_tracked_epoch` advanced to `epoch`.
    pub async fn on_finalized(
        &mut self,
        at: B256,
        number: u64,
    ) -> Result<TransitionOutcome, ReadError> {
        // `epochBlockInterval` is treated as FIXED after genesis: the consensus
        // `FixedEpocher` is frozen at startup, so acting on a live governance
        // change here would diverge the two epoch authorities (a boundary-synced
        // live re-interval is a separate, deferred task). Freeze on the first
        // finalized block; log + ignore any later on-chain change.
        let observed = self.reader.epoch_block_interval(at)?;
        if observed == 0 {
            return Err(ReadError::ZeroEpochInterval);
        }
        let interval = match self.frozen_interval {
            Some(frozen) => {
                if observed != frozen {
                    tracing::warn!(
                        frozen,
                        observed,
                        "epochBlockInterval changed on-chain but is treated as fixed \
                         after genesis (consensus FixedEpocher is frozen); ignoring"
                    );
                }
                frozen
            }
            None => {
                self.frozen_interval = Some(observed);
                observed
            }
        };
        // Freeze the relative-epoch origin on the first finalized block, mirroring
        // the interval freeze (consensus OriginEpocher is frozen at startup).
        let activation = {
            let observed = self.reader.dpos_activation_block(at)?;
            match self.frozen_activation {
                Some(frozen) => {
                    if observed != frozen {
                        tracing::warn!(
                            frozen,
                            observed,
                            "dposActivationBlock changed on-chain but is treated as fixed \
                             after genesis (consensus OriginEpocher is frozen); ignoring"
                        );
                    }
                    frozen
                }
                None => {
                    self.frozen_activation = Some(observed);
                    observed
                }
            }
        };
        let epoch_e = epoch_of_block(number, interval, activation);

        // Cold-start bootstrap: on the very first finalized block, stand up the
        // CURRENT epoch's engine. Its committee is already committed on-chain (the
        // ahead-commit pipeline committed it during the prior epoch), so read the
        // frozen array. `return` so a cold-start call never ALSO falls through to
        // the boundary branch below — otherwise an anchor on the last block of an
        // epoch whose `track_and_trigger` hit a Full channel (last_tracked stays
        // None → `None < Some(next)`) would double-spawn epoch E+1 while E was
        // never tracked.
        if self.last_tracked_epoch.is_none() {
            let snap = self.reader.epoch_committee_snapshot(epoch_e, at)?;
            if snap.validators.is_empty() {
                return Ok(TransitionOutcome::Intra);
            }
            return self.track_and_trigger(epoch_e, snap, at).await;
        }

        // Boundary: when the LAST block of epoch E finalizes, spawn epoch E+1. Its
        // committee was committed one epoch ahead (at the first block of epoch E,
        // §4.4), so the frozen `getEpochCommittee(E+1)` is on-chain by now and the
        // genesis block for engine E+1 (= this finalized last-block of E) is
        // stored. The engine-E engine keeps producing until E+1 takes over.
        let next = epoch_e + 1;
        // Boundary detection MUST be activation-relative, matching `epoch_of_block`
        // (reader.rs) and the consensus `OriginEpocher`: the last block of relative
        // epoch E is where `(number - activation) % interval == interval - 1`, i.e.
        // `(number + 1 - activation) % interval == 0`. The absolute form
        // `(number + 1) % interval == 0` only agrees when `activation % interval == 0`
        // (a devnet bootstrap convention, NOT enforced — prod cold-start anchors on
        // an arbitrary recent finalized height), so an absolute check would fire the
        // peer-set handoff at a different block than `OriginEpocher` treats as the
        // boundary — the exact "two epoch authorities diverge" failure the freeze
        // logic above guards against.
        let is_epoch_boundary = (number + 1)
            .saturating_sub(activation)
            .is_multiple_of(interval as u64);
        if is_epoch_boundary && self.last_tracked_epoch < Some(next) {
            // Missed-commit epoch: `Staking.sol` allows an epoch with no
            // `commitEpochCommittee` (unslashable by design; idempotent / monotonic
            // — a skip is safe); `getEpochCommittee` returns empty. Do NOT
            // persist/track an empty peer set — skip so a later finalized block can
            // still apply it if the commit lands, and commonware keeps the prior set.
            let snap = self.reader.epoch_committee_snapshot(next, at)?;
            if snap.validators.is_empty() {
                return Ok(TransitionOutcome::Intra);
            }
            return self.track_and_trigger(next, snap, at).await;
        }
        Ok(TransitionOutcome::Intra)
    }

    /// Persist + size-check + prune the frozen committee, feed the peer set to the
    /// sink, and fire the boundary trigger — advancing `last_tracked_epoch` only on
    /// a successful `try_send`. Extracted so both the cold-start bootstrap and the
    /// boundary branch share identical (idempotent) side effects.
    async fn track_and_trigger(
        &mut self,
        epoch: u64,
        snap: crate::reader::ValidatorSetSnapshot,
        at: B256,
    ) -> Result<TransitionOutcome, ReadError> {
        let keys: Vec<PeerPubkey> = snap
            .validators
            .iter()
            .map(|v| v.keys.peer_pubkey.clone())
            .collect();
        check_committee_size(epoch, keys.len(), self.max_peer_set_size)?; // committee-size guard (typed, not panic)
        let retention =
            self.reader.undelegate_period(at)? as u64 + EPOCH_COMMITTEE_RETENTION_MARGIN;
        {
            let mut cache = self.cache.lock().await;
            cache.persist_final(snap.clone()).await?; // finality-gated — idempotent
            cache.prune(epoch.saturating_sub(retention)).await?; // mirror on-chain prune
        }
        self.sink.track(epoch, Set::from_iter_dedup(keys)).await; // one-shot

        // Gate `last_tracked_epoch` advance on `try_send` success. A
        // `Full` channel means the consensus bridge is backed up; leave the
        // epoch un-tracked so the next finalized block re-enters here
        // (persist/track/prune are idempotent — see contract above), retries
        // the send, and only advances `last_tracked_epoch` once consensus
        // actually saw the boundary trigger. A `Closed` channel means the
        // forwarder shut down; the forwarder itself fires the shutdown_token
        // path (see crates/node/src/dpos.rs bridge forwarder), so we just
        // log and return Intra without advancing.
        if let Some(tx) = self.boundary_tx.as_ref() {
            match tx.try_send((epoch, snap)) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(epoch, "bridge channel full; retry on next finalized block");
                    return Ok(TransitionOutcome::Intra);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    tracing::error!(epoch, "bridge channel closed — forwarder has shut down");
                    return Ok(TransitionOutcome::Intra);
                }
            }
        }
        self.last_tracked_epoch = Some(epoch);
        Ok(TransitionOutcome::EpochAdvanced(epoch))
    }

    /// Cold start: read the **current finalized** committee at `head` and
    /// apply it **once**. No historical replay — there is no point taking an
    /// outdated state because the network has moved on, and anything older
    /// than the contract's retention window is pruned on-chain anyway.
    /// Delegates to the steady-state path.
    pub async fn cold_start(
        &mut self,
        head: B256,
        head_number: u64,
    ) -> Result<TransitionOutcome, ReadError> {
        self.on_finalized(head, head_number).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::{ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys};
    use alloy_primitives::Address;
    use commonware_codec::DecodeExt as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner};
    use fluentbase_bls::BlsPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;
    use std::sync::{Arc, Mutex};

    fn validator(seed: u64) -> ValidatorWithKeys {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer = Ed25519PrivateKey::random(&mut rng).public_key();
        let bls = BlsPubkey::decode(
            fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut rng)
                .public_bytes()
                .as_slice(),
        )
        .unwrap();
        ValidatorWithKeys {
            address: Address::repeat_byte(seed as u8),
            keys: ConsensusKeys {
                bls_pubkey: bls,
                peer_pubkey: peer,
                activation_epoch: 1,
            },
        }
    }

    /// Canned reader: fixed committee size + undelegate period + interval.
    struct MockReader {
        committee: usize,
        undelegate: u32,
        interval: u32,
    }
    impl StakingStateRead for MockReader {
        fn epoch_committee_snapshot(
            &self,
            epoch: u64,
            at: B256,
        ) -> Result<ValidatorSetSnapshot, ReadError> {
            Ok(ValidatorSetSnapshot {
                block_hash: at,
                block_number: epoch * 100,
                epoch,
                validators: (0..self.committee as u64)
                    .map(|i| validator(epoch * 1000 + i))
                    .collect(),
            })
        }
        fn undelegate_period(&self, _at: B256) -> Result<u32, ReadError> {
            Ok(self.undelegate)
        }
        fn epoch_block_interval(&self, _at: B256) -> Result<u32, ReadError> {
            Ok(self.interval)
        }
        fn dpos_activation_block(&self, _at: B256) -> Result<u64, ReadError> {
            Ok(0) // mock tests use absolute numbering
        }
    }

    /// Records every `track` call.
    #[derive(Clone, Default)]
    struct RecordingSink(Arc<Mutex<Vec<(u64, usize)>>>);
    impl PeerSetSink for RecordingSink {
        fn track(&mut self, epoch: u64, peers: Set<PeerPubkey>) -> impl Future<Output = ()> + Send {
            let log = self.0.clone();
            async move {
                log.lock().unwrap().push((epoch, peers.len()));
            }
        }
    }

    #[test]
    fn boundary_apply_persists_and_tracks_once() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                sink.clone(),
                64,
                None,
            );
            let h = B256::repeat_byte(0x11);
            // block 500, interval 100 ⇒ epoch 5: first (cold-start) call bootstraps
            // the current epoch ⇒ EpochAdvanced(5)
            let outcome_first = et.on_finalized(h, 500).await.unwrap();
            assert_eq!(outcome_first, TransitionOutcome::EpochAdvanced(5));
            // re-delivery on a MID-epoch block (550 is not the last block of epoch
            // 5, so it is not a boundary) ⇒ Intra. (599 would be the last block of
            // epoch 5 and now legitimately spawns epoch 6 — see the boundary test.)
            let outcome_second = et.on_finalized(h, 550).await.unwrap();
            assert_eq!(outcome_second, TransitionOutcome::Intra);
            {
                let log = sink.0.lock().unwrap();
                assert_eq!(*log, vec![(5, 5)], "tracked once, 5 peers, epoch 5");
            }
            assert!(
                et.cache.lock().await.contains(h).await.unwrap(),
                "snapshot persisted"
            );
        });
    }

    #[test]
    fn last_block_of_epoch_spawns_next_epoch() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 5,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                sink.clone(),
                64,
                None,
            );
            let h = B256::repeat_byte(0x22);
            // cold-start mid-epoch-5 ⇒ bootstrap epoch 5
            assert_eq!(
                et.on_finalized(h, 550).await.unwrap(),
                TransitionOutcome::EpochAdvanced(5)
            );
            // last block of epoch 5 ((599+1)%100==0) ⇒ spawn epoch 6 one ahead
            assert_eq!(
                et.on_finalized(h, 599).await.unwrap(),
                TransitionOutcome::EpochAdvanced(6)
            );
            // mid-epoch-6 re-delivery ⇒ Intra (already tracked 6)
            assert_eq!(
                et.on_finalized(h, 650).await.unwrap(),
                TransitionOutcome::Intra
            );
            let log = sink.0.lock().unwrap();
            assert_eq!(*log, vec![(5, 5), (6, 5)], "bootstrap epoch 5, then spawn epoch 6 at its boundary");
        });
    }

    #[test]
    fn oversize_committee_is_typed_error_not_panic() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 10,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                RecordingSink::default(),
                4, // max_peer_set_size < committee
                None,
            );
            assert!(matches!(
                et.on_finalized(B256::repeat_byte(0x22), 200).await,
                Err(ReadError::CommitteeTooLarge {
                    epoch: 2,
                    size: 10,
                    max: 4
                })
            ));
        });
    }

    #[test]
    fn zero_interval_is_typed_error_not_panic() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 0,
                },
                cache,
                RecordingSink::default(),
                64,
                None,
            );
            assert!(matches!(
                et.on_finalized(B256::repeat_byte(0x01), 100).await,
                Err(ReadError::ZeroEpochInterval)
            ));
        });
    }

    #[test]
    fn missed_commit_epoch_skipped_not_tracked_empty() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 0,
                    undelegate: 7,
                    interval: 100,
                }, // no commit ⇒ empty
                cache,
                sink.clone(),
                64,
                None,
            );
            let h = B256::repeat_byte(0x44);
            // epoch 7, empty ⇒ Intra (empty-committee is a no-op, not an advance)
            let outcome = et.on_finalized(h, 700).await.unwrap();
            assert_eq!(outcome, TransitionOutcome::Intra);
            assert!(
                sink.0.lock().unwrap().is_empty(),
                "no empty peer set tracked"
            );
            assert!(
                !et.cache.lock().await.contains(h).await.unwrap(),
                "empty snapshot not persisted"
            );
            assert_eq!(et.last_tracked_epoch, None, "epoch NOT write-once-locked");
        });
    }

    #[test]
    fn try_send_full_returns_intra_and_does_not_advance() {
        // When the bridge channel is full, on_finalized must leave
        // last_tracked_epoch un-advanced so the next finalized block retries.
        // Outcome must be `Intra` so the dpos.rs hook does NOT reset its
        // consecutive-error counter.
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            // Capacity-1 channel; pre-fill it so try_send returns Full on
            // the next attempt without needing a real consumer.
            let (boundary_tx, _boundary_rx) = tokio::sync::mpsc::channel(1);
            // Pre-fill: take a fake (epoch, snap) slot.
            let dummy = ValidatorSetSnapshot {
                block_hash: B256::ZERO,
                block_number: 0,
                epoch: 999,
                validators: vec![],
            };
            boundary_tx.try_send((999, dummy)).expect("first slot");
            // Now channel is full.
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                sink.clone(),
                64,
                Some(boundary_tx),
            );
            let h = B256::repeat_byte(0xC6);
            let outcome = et.on_finalized(h, 500).await.unwrap(); // epoch 5
            assert_eq!(
                outcome,
                TransitionOutcome::Intra,
                "Full bridge channel must surface as Intra outcome"
            );
            assert_eq!(
                et.last_tracked_epoch, None,
                "last_tracked_epoch must NOT advance"
            );
        });
    }

    #[test]
    fn boundary_tx_fires_once_per_epoch() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let (boundary_tx, mut boundary_rx) = tokio::sync::mpsc::channel(8);
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 4,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                sink.clone(),
                64,
                Some(boundary_tx),
            );
            let h = B256::repeat_byte(0xCD);
            et.on_finalized(h, 800).await.unwrap();
            et.on_finalized(h, 850).await.unwrap();
            let first = boundary_rx.try_recv().expect("first boundary fires");
            assert_eq!(first.0, 8);
            assert_eq!(first.1.validators.len(), 4);
            assert!(boundary_rx.try_recv().is_err(), "no duplicate boundary");
        });
    }

    #[test]
    fn cold_start_reads_current_finalized_once() {
        deterministic::Runner::default().start(|ctx| async move {
            let cache = std::sync::Arc::new(tokio::sync::Mutex::new(
                ValidatorSetCache::init(ctx).await.unwrap(),
            ));
            let sink = RecordingSink::default();
            let mut et = EpochTransition::new(
                MockReader {
                    committee: 3,
                    undelegate: 7,
                    interval: 100,
                },
                cache,
                sink.clone(),
                64,
                None,
            );
            et.cold_start(B256::repeat_byte(0x33), 1200).await.unwrap();
            assert_eq!(*sink.0.lock().unwrap(), vec![(12, 3)]);
        });
    }
}
