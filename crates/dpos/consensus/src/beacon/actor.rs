//! Networked live-DKG actor: wraps [`DkgCeremony`] and drives committee[E]'s
//! self-DKG over `BEACON_CHANNEL` during epoch E-1.
//!
//! Single-ceremony-per-epoch, NO Muxer: each `DkgMsg` carries its `ceremony_epoch`.
//! A dealing (`Commitment`/`Share`) that arrives for a near-future epoch BEFORE this
//! node started its own ceremony for it is BUFFERED (`pending`, drained by
//! `maybe_start`) so the start-race never silently drops it; any other message not for
//! an active ceremony is dropped (epoch-tag filter). Ceremonies for E and E+1 are
//! temporally disjoint (the collection window spans ~all of E-1), so at most a couple
//! are in flight.
//!
//! Lifecycle, driven by the finalized-height stream + chain committee reads:
//! - entering epoch E-1 (committee[E] != committee[E-1] AND this node ∈
//!   committee[E]) → `DkgCeremony::start`, broadcast commitment + send shares;
//! - finalized height reaches `epoch_start(E) - DKG_MARGIN_BLOCKS` → `seal_dealings`
//!   (broadcast the signed log);
//! - once a sealed ceremony has a selectable quorum AND its log set has SETTLED
//!   (all-in, or the deterministic settle deadline — so every honest node selects the
//!   IDENTICAL set ⇒ identical `PK_E`; probed event-driven on each recording, our seal
//!   or an incoming `Reveal`, via [`DkgActor::drive_finalization`]) →
//!   `DkgCeremony::finalize` → memoize `(PK_E, share)` into the per-epoch
//!   [`CeremonyStore`] + fire `share_notify`. The consensus verify
//!   path reads the share for the C share-on-polynomial gate, `propose` reads
//!   `PK_E` for the boundary `beacon_outcome`, and Phase 5's finalized-boundary
//!   swap reads both for the per-epoch signing slot + `commitEpochBeaconKey`.
//!
//! The actor never finalizes over a locally-selected Q before sealing, and never
//! over an under-quorum log set (`ready` gates it). <quorum valid logs → no store
//! entry → the beacon naturally stalls for that epoch (option A), not a crash.

use crate::beacon::{
    ceremony::{CeremonyOutput, DkgCeremony, Outgoing, Target},
    dkg_msg::{DkgBody, DkgMsg},
    wire::BeaconMessage,
};
use commonware_codec::{Encode as _, Read as _, ReadExt as _};
use commonware_cryptography::{
    bls12381::primitives::group::Share, ed25519::PrivateKey as Ed25519PrivateKey, Signer as _,
};
use commonware_p2p::{Receiver, Recipients, Sender};
use commonware_utils::ordered::Set;
use fluentbase_bls::PeerPubkey;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    path::PathBuf,
    sync::{Arc, RwLock},
};

/// Blocks of slack before the epoch-E boundary at which dealing collection closes
/// and dealers seal (broadcast their signed logs) — the echo-settle tail. Pinned
/// off the on-chain `epochBlockInterval`, not an absolute window (see Q4).
pub const DKG_MARGIN_BLOCKS: u64 = 10;

/// Blocks AFTER the seal deadline to keep collecting peer logs before finalizing
/// over whatever valid set has settled — the deterministic fallback when a dealer is
/// genuinely absent (the all-present case finalizes earlier, the instant the last
/// `Reveal` lands). Bounds the canonical-set wait so every honest node selects over
/// the IDENTICAL log set at the identical height ⇒ identical `PK_E` ⇒ the C gate
/// passes. MUST stay `< DKG_MARGIN_BLOCKS` so finalize lands before the boundary.
pub const DKG_SETTLE_BLOCKS: u64 = 4;
const _: () = assert!(
    DKG_SETTLE_BLOCKS < DKG_MARGIN_BLOCKS,
    "settle window must finalize before the epoch boundary"
);

/// The epoch the beacon goes live at, deterministically. `committee[2]` runs its
/// DKG during epoch 1 EVEN IF unchanged from `committee[1]`, so a long-stable
/// initial committee still seeds the beacon (on-change-only activation would
/// leave it seedless indefinitely). Epoch 1 stays seedless (`order.digest()`);
/// on-change re-DKG + carry-forward apply thereafter. The same constant gates the
/// `application::is_change_epoch_first_block` boundary so the two never drift.
pub const DETERMINISTIC_BOOTSTRAP_EPOCH: u64 = 2;

/// The agreed DKG result for an epoch this node is a MEMBER of: the group output
/// (`PK_E` + public polynomial) and this node's secret share, memoized by the
/// actor during the post-seal margin window — BEFORE the epoch-E boundary block
/// is proposed/verified. Read by the consensus verify/propose path (the C gate's
/// share + the proposer's `beacon_outcome`) and by Phase 5's signing-slot swap at
/// the finalized boundary. Non-members never get an entry (⇒ observer ⇒ withhold).
pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, (CeremonyOutput, Share)>>>;

/// Resolves committee[epoch] (the Commonware-ordered peer set) at a finalized
/// state hash — provided by the launch site over the staking reader.
pub type CommitteeFor = Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>;

/// The networked DKG actor. Generic over the p2p sender/receiver so the spawn site
/// passes the `BEACON_CHANNEL` halves; testable with mock channels.
pub struct DkgActor<Se, Re> {
    namespace: Vec<u8>,
    me_key: Ed25519PrivateKey,
    sender: Se,
    receiver: Re,
    committee_for: CommitteeFor,
    store: CeremonyStore,
    /// Edge-trigger fired (`notify_one`) the instant a share lands in `store`, so a
    /// racing `epoch_manager::enter` wakes immediately instead of polling. The SAME
    /// `Arc` is held by the manager (threaded via `SharedBeaconPlane`).
    share_notify: Arc<tokio::sync::Notify>,
    /// Frozen `(dposActivationBlock, epochBlockInterval)` — the immutable epoch
    /// geometry, resolved ONCE by the beacon plane's `EpochTransition` (the single
    /// in-plane source) and handed in as plain values at spawn AFTER that freeze.
    /// The actor never re-reads the chain for it, so there is no codeless/genesis-
    /// fallback race in this path: the spawn site only constructs the actor once the
    /// geometry is frozen (see `build_beacon_plane`).
    dpos_activation: u64,
    epoch_interval: u64,
    metrics: crate::beacon::metrics::BeaconMetrics,
    /// Directory for on-disk share persistence (item A): the parent of
    /// `--dpos.beacon-share-path`. `None` ⇒ no share-path configured (e.g. a
    /// case-(b) sharing-only node) ⇒ memoized shares stay in-memory only.
    share_dir: Option<PathBuf>,
    /// Active ceremonies keyed by their target epoch E.
    ceremonies: BTreeMap<u64, DkgCeremony>,
    /// Dealings (`Commitment`/`Share`) that arrived for an epoch BEFORE this node
    /// started its own ceremony for it — the start-race. Drained into the ceremony by
    /// `maybe_start` before any seal, so a peer dealing that raced ahead of our start
    /// is never silently dropped (which would leave that dealer un-acked ⇒
    /// `TooManyReveals` ⇒ `DkgFailed`). Bounded: only the next 1–2 epochs are
    /// bufferable (`is_bufferable`), ≤ `MAX_COMMITTEE_SIZE * 2` entries per epoch, and
    /// stale epochs are evicted each height tick.
    pending: BTreeMap<u64, Vec<(PeerPubkey, DkgBody)>>,
    /// Target epochs whose dealing phase has been sealed.
    sealed: BTreeSet<u64>,
    /// Last finalized height seen on the `on_height` stream — the current chain time
    /// the event-driven `on_message` finalize uses for its deterministic-settle gate.
    last_height: u64,
    /// Target epochs whose committee first-became-readable has been logged
    /// (one-shot diagnostic; see `maybe_start`).
    eval_logged: BTreeSet<u64>,
}

impl<Se, Re> DkgActor<Se, Re>
where
    Se: Sender<PublicKey = PeerPubkey>,
    Re: Receiver<PublicKey = PeerPubkey>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: Vec<u8>,
        me_key: Ed25519PrivateKey,
        sender: Se,
        receiver: Re,
        committee_for: CommitteeFor,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        dpos_activation: u64,
        epoch_interval: u64,
        metrics: crate::beacon::metrics::BeaconMetrics,
        share_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            namespace,
            me_key,
            sender,
            receiver,
            committee_for,
            store,
            share_notify,
            dpos_activation,
            epoch_interval,
            metrics,
            share_dir,
            ceremonies: BTreeMap::new(),
            pending: BTreeMap::new(),
            sealed: BTreeSet::new(),
            last_height: 0,
            eval_logged: BTreeSet::new(),
        }
    }

    fn epoch_of(&self, height: u64) -> u64 {
        height.saturating_sub(self.dpos_activation) / self.epoch_interval
    }

    /// First-block height of an epoch (relative to DPoS activation).
    fn epoch_start(&self, epoch: u64) -> u64 {
        self.dpos_activation + epoch * self.epoch_interval
    }

    /// Run until both the height-event stream and the network receiver close.
    /// `heights` carries every finalized block height (tapped from the boundary
    /// hook); the actor derives epoch transitions + the seal deadline from it.
    pub async fn run(mut self, mut heights: tokio::sync::mpsc::Receiver<u64>, mut rng: impl CryptoRngCore) {
        tracing::info!(
            activation = self.dpos_activation,
            interval = self.epoch_interval,
            "live DKG: actor started"
        );
        loop {
            tokio::select! {
                maybe_h = heights.recv() => match maybe_h {
                    Some(height) => self.on_height(height, &mut rng).await,
                    None => break,
                },
                msg = self.receiver.recv() => match msg {
                    Ok((from, buf)) => self.on_message(from, buf.as_ref(), &mut rng).await,
                    Err(_) => break,
                },
            }
        }
    }

    async fn on_height(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // Two feeders drive this clock: the local finalized-height poller
        // (`fin + K`) and, during unified-supervisor catch-up, the LIVE upstream
        // cert frontier (so a still-catching-up newcomer deals its first epoch on
        // the live deadline). Take the max so an interleaved lagging tick can never
        // pull the deal/seal clock backward; process at the monotone height.
        self.last_height = self.last_height.max(height);
        let height = self.last_height;
        let now = self.epoch_of(height);
        let mut to_send: Vec<Outgoing> = Vec::new();

        // 1. Seal any active ceremony whose collection deadline has passed.
        let due: Vec<u64> = self
            .ceremonies
            .keys()
            .copied()
            .filter(|e| {
                !self.sealed.contains(e)
                    && height >= self.epoch_start(*e).saturating_sub(DKG_MARGIN_BLOCKS)
            })
            .collect();
        for e in due {
            if let Some(c) = self.ceremonies.get_mut(&e) {
                to_send.extend(c.seal_dealings());
                self.sealed.insert(e);
            }
        }

        // 1b. Evict pending dealing buffers for epochs we will never start (already
        //     sealed, or now in the past) so `pending` stays O(1–2 live epochs).
        self.pending.retain(|e, _| *e > now && !self.sealed.contains(e));

        // 2. Finalize any SEALED ceremony whose log set has SETTLED (all-in, or the
        //    deterministic settle deadline). Also driven from `on_message`, so an
        //    all-in completed by an incoming Reveal finalizes immediately — see
        //    [`Self::drive_finalization`].
        self.drive_finalization(height, rng);

        // 2b. Evict any ceremony/`sealed` entry whose epoch boundary has passed without
        //     finalizing. An under-quorum stall still gets SEALED in step 1 but never
        //     reaches `ready()`, so `drive_finalization` never removes it — without this
        //     sweep it lingers in `ceremonies`/`sealed` and re-probes `ready()` (an n=51
        //     `observe` over cloned logs) on every height tick AND every incoming message
        //     forever. Past its own boundary (`*e <= now`) the ceremony can only ever be
        //     the option-A no-op (the boundary block already needed `PK_e` and is gone),
        //     so dropping it is safe. Runs AFTER `drive_finalization` so a ceremony that
        //     is finalizable on the boundary tick is completed first, never evicted out
        //     from under it. Mirrors the `pending.retain` sweep above.
        self.ceremonies.retain(|e, _| *e > now);
        self.sealed.retain(|e| *e > now);

        // 3. Start the NEXT epoch's DKG ceremony. Retried on EVERY tick (not just
        //    once at the epoch transition): committee[E+1] is committed on-chain
        //    sometime DURING epoch E, which can land AFTER the actor (driven by
        //    lagging finalized heights) first enters E — a single-shot check at the
        //    transition would see the committee still unchanged, carry forward, and
        //    NEVER deal, so the E+1 boundary block wedges (no PK_{E+1}). maybe_start
        //    is idempotent (no-op once the ceremony is in flight, already computed,
        //    the committee is unchanged, or this node is not a member), so retrying
        //    until committee[E+1] is visible+changed is safe.
        self.maybe_start(now + 1, rng, &mut to_send);

        self.broadcast_all(to_send).await;
    }

    /// Finalize every SEALED ceremony whose collected log set has SETTLED, memoizing
    /// `(PK_E, share)` into the shared [`CeremonyStore`] over a DETERMINISTIC canonical
    /// log set.
    ///
    /// Event-driven, not clock-polled. A ceremony's log set grows ONLY on our own
    /// `seal_dealings` or an incoming `Reveal` (`DkgCeremony::handle` → `Logs::record`),
    /// so this runs from exactly those two events: `on_height` after sealing, and
    /// `on_message` after handling a peer message. No timer.
    ///
    /// DETERMINISM is the load-bearing property. `select` is already canonical (the
    /// `required_commitments` lowest-keyed valid dealers — a total order every node
    /// shares), so `finalize` yields the byte-identical `PK_E` PROVIDED every honest
    /// node selects over the IDENTICAL log set. The settle gate (in the filter below)
    /// enforces that: finalize fires only once the set is all-in (every committee log
    /// recorded) or the height-deterministic settle deadline has passed. Without it,
    /// nodes finalizing at their FIRST selectable quorum pick DIFFERENT subsets ⇒
    /// divergent `PK_E` ⇒ the boundary "C" share-on-poly gate
    /// (`application::beacon_gate_decision`) rejects (the observed `vrf` wedge). With
    /// it, honest divergence is eliminated by construction; the n=51
    /// `dkg::seed_is_threshold_unique_at_n51` test confirms determinism at production
    /// committee size.
    ///
    /// The `self.sealed` guard preserves the seal-before-finalize contract: a peer
    /// `Reveal` recorded before our own local seal cannot trigger a premature finalize.
    /// Idempotent: `finalize` consumes the ceremony, so a later trigger for the same
    /// epoch is a no-op. `< required_commitments` valid logs ever settling ⇒ `ready`
    /// stays false ⇒ never finalized ⇒ the natural option-A stall (the residual
    /// LIVENESS-only failure, paired with `dkg_ceremony_fail_total` below; a
    /// forged/divergent `PK_E` is independently caught by the Stage-2 certify hook
    /// `beacon::certify`, which σ-verifies the recovered seed and Nullifies on
    /// mismatch). The Byzantine log-equivocation case (a dealer signing conflicting
    /// logs) is the still-deferred consensus-pinned-QUAL residual
    /// (`dpos_beacon_share_reshare`). The actor is single-threaded (`run`'s `select!`),
    /// so there is no concurrent mutation of `ceremonies`/`sealed`.
    fn drive_finalization(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
        // `ready` probes non-destructively (Logs clone); `finalize` then consumes the
        // fulfilled ceremony. Both run STILL DURING the margin window — before the
        // epoch's boundary block is proposed/verified — so the verify-path C gate can
        // read the share.
        let ready: Vec<u64> = self
            .ceremonies
            .iter()
            .filter(|(e, c)| {
                if !self.sealed.contains(*e) || !c.ready(rng) {
                    return false;
                }
                // Finalize only over a SETTLED set so every honest node selects the
                // IDENTICAL log set (→ identical canonical `select` → identical PK_E →
                // the C gate passes): either ALL-IN (every committee log recorded —
                // the common case, the instant the last Reveal lands) or the
                // height-deterministic SETTLE deadline past the seal (the shorthanded
                // fallback — an absent dealer never reveals, so the n−f survivors
                // finalize over the settled present set instead of waiting forever).
                let target = **e;
                let n = (self.committee_for)(target).map_or(0, |s| s.len());
                let all_in = n > 0 && c.recorded_log_count() == n;
                let seal_deadline = self.epoch_start(target).saturating_sub(DKG_MARGIN_BLOCKS);
                let settle_due = height >= seal_deadline + DKG_SETTLE_BLOCKS;
                all_in || settle_due
            })
            .map(|(e, _)| *e)
            .collect();
        for e in ready {
            let Some(c) = self.ceremonies.remove(&e) else {
                continue;
            };
            self.sealed.remove(&e);
            match c.finalize(rng) {
                Ok((out, share)) => {
                    // Item A: persist (PK_E, share) to disk BEFORE the in-memory
                    // insert (which moves the pair), so a mid-epoch restart reloads
                    // it instead of carry-forwarding the wrong key and stalling.
                    // Best-effort — the in-memory store is authoritative for the
                    // running process, so a write failure only warns.
                    if let Some(dir) = &self.share_dir {
                        if let Err(err) = crate::beacon::share_state::persist(dir, e, &out, &share) {
                            tracing::warn!(
                                epoch = e,
                                ?err,
                                "live DKG: failed to persist share to disk (in-memory store unaffected)"
                            );
                        }
                    }
                    if let Ok(mut store) = self.store.write() {
                        store.insert(e, (out, share));
                    }
                    // Edge-trigger the boundary-entry waiter: the share is now visible
                    // in the store, so a racing `enter(e)` wakes immediately rather than
                    // polling. `notify_one` stores a permit when no waiter is armed, so a
                    // share that lands between the consumer's reconcile and its re-arm is
                    // not lost (single consumer: `EpochManager::run`).
                    self.share_notify.notify_one();
                    self.metrics.dkg_ceremony_ok.inc();
                    tracing::info!(epoch = e, "live DKG: PK_epoch + share computed + stored");
                }
                Err(err) => {
                    self.metrics.dkg_ceremony_fail.inc();
                    tracing::warn!(
                        epoch = e,
                        ?err,
                        "live DKG: finalize failed after ready-probe — beacon stalls for this epoch"
                    );
                }
            }
        }
    }

    /// Start a ceremony for `target` (run during the just-entered epoch) when the
    /// committee actually changes; an unchanged committee carries the key forward
    /// (no ceremony — Phase 5 reuses the prior epoch's `BeaconKey`).
    fn maybe_start(&mut self, target: u64, rng: &mut impl CryptoRngCore, out: &mut Vec<Outgoing>) {
        // Skip if a ceremony for this epoch is in flight OR already computed (the
        // per-tick retry would otherwise re-deal an epoch whose ceremony finished
        // and was removed from `ceremonies`).
        if target == 0 || self.ceremonies.contains_key(&target) {
            return;
        }
        if self
            .store
            .read()
            .ok()
            .is_some_and(|s| s.contains_key(&target))
        {
            return;
        }
        let cur = (self.committee_for)(target - 1);
        let next = (self.committee_for)(target);
        let me = self.me_key.public_key();
        // One-shot diagnostic: log when committee[target] FIRST becomes readable,
        // with the deal decision inputs — pinpoints start vs carry-forward vs
        // not-member vs committee-never-readable without per-tick spam.
        if next.is_some() && self.eval_logged.insert(target) {
            tracing::info!(
                target,
                cur_n = cur.as_ref().map(|c| c.len()),
                next_n = next.as_ref().map(|c| c.len()),
                change = (cur.as_ref() != next.as_ref()),
                me_member = next.as_ref().is_some_and(|n| n.iter().any(|p| *p == me)),
                "live DKG: committee[target] readable — maybe_start eval"
            );
        }
        let (Some(cur), Some(next)) = (cur, next) else {
            return;
        };
        // Deterministic epoch-2 bootstrap: committee[2] always deals (during epoch
        // 1) even when unchanged, so a long-stable initial committee still seeds the
        // beacon. Every other epoch carries the key forward on an unchanged committee.
        if next == cur && target != DETERMINISTIC_BOOTSTRAP_EPOCH {
            return; // carry-forward
        }
        // Model B: only a MEMBER of committee[target] deals to itself. A node that
        // is in committee[target-1] but not committee[target] does not deal.
        if !next.iter().any(|p| *p == me) {
            return;
        }
        match DkgCeremony::start(rng, &self.namespace, target, next, self.me_key.clone()) {
            Ok((ceremony, outgoing)) => {
                self.ceremonies.insert(target, ceremony);
                out.extend(outgoing);
                // Drain any dealings that raced ahead of our start (the start-race):
                // replay them through `handle` NOW, before any seal, so every dealer
                // we heard from is acked. Order-independent (`try_ack` fires only once
                // both halves are buffered). The acks the replay emits are collected
                // into `out` and broadcast by the caller, so a dealer that previously
                // got ≤ quorum−1 acks now seals `Ok`, not `TooManyReveals`.
                if let Some(buffered) = self.pending.remove(&target) {
                    let c = self.ceremonies.get_mut(&target).expect("just inserted");
                    for (from, body) in buffered {
                        out.extend(c.handle(from, body));
                    }
                }
                tracing::info!(epoch = target, "live DKG: ceremony started");
            }
            Err(e) => tracing::warn!(epoch = target, ?e, "live DKG: ceremony start failed"),
        }
    }

    /// Whether an incoming DKG message should be BUFFERED when no ceremony for its
    /// epoch exists yet (the start-race), rather than dropped. Only DEALINGS
    /// (`Commitment`/`Share`) for a near-future, not-yet-sealed, not-yet-finalized
    /// epoch qualify — acks/reveals are meaningless without a live ceremony to feed,
    /// and `last_height` bounds the future window so far-future / garbage epochs
    /// cannot accumulate (a DoS guard); stale buffers are also evicted each tick.
    fn is_bufferable(&self, epoch: u64, body: &DkgBody) -> bool {
        if !matches!(body, DkgBody::Commitment(_) | DkgBody::Share(_)) {
            return false;
        }
        if self.sealed.contains(&epoch) {
            return false;
        }
        let now = self.epoch_of(self.last_height);
        if epoch <= now || epoch > now + 2 {
            return false; // already started / past, or too far in the future
        }
        self.store.read().map_or(true, |s| !s.contains_key(&epoch))
    }

    async fn on_message(&mut self, from: PeerPubkey, buf: &[u8], rng: &mut impl CryptoRngCore) {
        // Decode bounded by MAX_COMMITTEE_SIZE (upper bound; exact n not needed).
        let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
            .expect("MAX_COMMITTEE_SIZE > 0");
        let mut wire = buf;
        let BeaconMessage::Dkg(payload) = match BeaconMessage::read(&mut wire) {
            Ok(m) => m,
            Err(_) => return,
        };
        let mut body = payload.as_ref();
        let msg = match DkgMsg::read_cfg(&mut body, &max) {
            Ok(m) => m,
            Err(_) => return,
        };
        // Epoch-tag filter. An active ceremony processes the message directly. A
        // DEALING for an epoch we have not started yet is a start-race victim — buffer
        // it (drained by `maybe_start`) rather than DROP it (a dropped dealing leaves
        // that dealer un-acked ⇒ `TooManyReveals` ⇒ `DkgFailed`). Acks/Reveals with no
        // live ceremony are still dropped (nothing to feed; a re-sealed Reveal
        // re-arrives once we are live via the long-window mechanism).
        if let Some(c) = self.ceremonies.get_mut(&msg.ceremony_epoch) {
            let out = c.handle(from, msg.body);
            self.broadcast_all(out).await;
            // A recorded Reveal may have just made a sealed ceremony all-in — finalize
            // NOW, event-driven, over the settled set. See [`Self::drive_finalization`].
            self.drive_finalization(self.last_height, rng);
        } else if self.is_bufferable(msg.ceremony_epoch, &msg.body) {
            let buf = self.pending.entry(msg.ceremony_epoch).or_default();
            if buf.len() < fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize * 2 {
                buf.push((from, msg.body));
            }
        }
    }

    /// Send each outgoing ceremony message over BEACON_CHANNEL (broadcast or direct).
    async fn broadcast_all(&mut self, msgs: Vec<Outgoing>) {
        for o in msgs {
            let wire = BeaconMessage::Dkg(o.msg.encode()).encode();
            let recipients = match o.target {
                Target::Broadcast => Recipients::All,
                Target::Direct(pk) => Recipients::One(pk),
            };
            // Best-effort: a dropped DKG message is recovered by the long window /
            // reveal mechanism; never block consensus on a send failure.
            let _ = self.sender.send(recipients, wire, false).await;
        }
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    use commonware_p2p::{
        simulated::{Config as SimConfig, Link, Network, Oracle},
        Manager as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Clock as _, Metrics as _, Runner as _, Spawner as _};
    use commonware_utils::NZUsize;
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::time::Duration;

    type SimContext = deterministic::Context;

    const SEAL_DEADLINE: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH - DKG_MARGIN_BLOCKS; // 10
    const BOUNDARY: u64 = INTERVAL * DETERMINISTIC_BOOTSTRAP_EPOCH; // 20 = epoch_start(2)
    const INTERVAL: u64 = 10;
    const ACTIVATION: u64 = 0;
    /// Result-final lag (the EL-finalized clock trails the ordering clock by this).
    const K: u64 = crate::K;

    /// Spawn one [`DkgActor`] over the simulated network and return its height sink.
    /// The actor deals `committee[2]` (the deterministic bootstrap epoch) against the
    /// other dealers; the stable committee makes every epoch carry-forward EXCEPT 2.
    async fn spawn_dealer(
        ctx: &SimContext,
        oracle: &Oracle<PeerPubkey, SimContext>,
        me: Ed25519PrivateKey,
        committee: Set<PeerPubkey>,
        store: CeremonyStore,
        share_notify: Arc<tokio::sync::Notify>,
        interval: u64,
    ) -> tokio::sync::mpsc::Sender<u64> {
        let pk = me.public_key();
        let (sender, receiver) = oracle
            .control(pk.clone())
            .register(
                fluentbase_p2p::constants::BEACON_CHANNEL,
                fluentbase_p2p::constants::BEACON_QUOTA,
            )
            .await
            .expect("register BEACON_CHANNEL");
        let committee_for: CommitteeFor = {
            let set = committee.clone();
            Arc::new(move |_epoch: u64| Some(set.clone()))
        };
        let actor = DkgActor::new(
            b"FLUENT_DPOS_V1_clocktest".to_vec(),
            me,
            sender,
            receiver,
            committee_for,
            store,
            share_notify,
            ACTIVATION,
            interval,
            crate::beacon::metrics::BeaconMetrics::default(),
            None,
        );
        let (height_tx, height_rx) = tokio::sync::mpsc::channel::<u64>(256);
        let rng = StdRng::seed_from_u64(7);
        drop(
            ctx.with_label("dealer")
                .spawn(move |_c| async move { actor.run(height_rx, rng).await }),
        );
        height_tx
    }

    /// Run the 4-dealer committee[2] DKG over the simulated network, driving the
    /// height clock with `lag` (0 = ordering clock; K = EL-finalized clock). Feeds
    /// one height per virtual tick up to and including `feed_to`; returns whether the
    /// victim memoized `(PK_2, share)` by then.
    async fn dkg_seeded_by(ctx: SimContext, lag: u64, feed_to: u64) -> bool {
        // freeze_at == feed_to ⇒ feed the whole range; no late starters.
        dkg_seeded_with_freeze(ctx, lag, feed_to, feed_to, INTERVAL, 0, 0).await
    }

    /// The 4-dealer committee[2] DKG over the sim network, returning whether the victim
    /// (node 0) memoized `(PK_2, share)`. Knobs:
    /// - `lag`: subtract from every fed height (0 = ordering clock, K = EL-finalized).
    /// - `freeze_at`: STOP feeding heights past this (still ticking) — models the
    ///   boundary stall; a victim seeding past it proves event-driven finalize.
    /// - `interval`: epoch length (a larger value gives a real dealing window).
    /// - `late_count`/`late_lag`: the first `late_count` nodes start `late_lag` ticks
    ///   LATE, so peers' dealings reach them BEFORE their own `maybe_start` — the
    ///   start-race. Those dealings must be BUFFERED (drained on start) and acked, not
    ///   dropped (a drop leaves the dealer un-acked ⇒ `TooManyReveals` ⇒ `DkgFailed`).
    async fn dkg_seeded_with_freeze(
        ctx: SimContext,
        lag: u64,
        feed_to: u64,
        freeze_at: u64,
        interval: u64,
        late_count: usize,
        late_lag: u64,
    ) -> bool {
        let oracle: Oracle<PeerPubkey, SimContext> = {
            let (network, oracle) = Network::new(
                ctx.with_label("sim_net"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            oracle
        };

        let mut rng = StdRng::seed_from_u64(1);
        let keys: Vec<Ed25519PrivateKey> =
            (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        oracle.manager().track(0, committee.clone()).await;
        for a in &keys {
            for b in &keys {
                if a.public_key() != b.public_key() {
                    oracle
                        .add_link(
                            a.public_key(),
                            b.public_key(),
                            Link {
                                latency: Duration::from_millis(0),
                                jitter: Duration::from_millis(0),
                                success_rate: 1.0,
                            },
                        )
                        .await
                        .expect("link");
                }
            }
        }

        let victim_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
        let mut sinks = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let store = if i == 0 {
                victim_store.clone()
            } else {
                Arc::new(RwLock::new(BTreeMap::new()))
            };
            let notify = Arc::new(tokio::sync::Notify::new());
            sinks.push(
                spawn_dealer(&ctx, &oracle, k.clone(), committee.clone(), store, notify, interval)
                    .await,
            );
        }

        for h in 0..=feed_to {
            for (i, s) in sinks.iter().enumerate() {
                // The first `late_count` nodes start LATE (their height feed lags by
                // `late_lag`), so peers' dealings arrive before their `maybe_start`.
                let node_h = if i < late_count {
                    h.saturating_sub(late_lag)
                } else {
                    h
                };
                // Past `freeze_at` we stop feeding (the boundary-stall freeze) but keep
                // ticking, so the sim delivers in-flight Reveals.
                if node_h <= freeze_at {
                    let _ = s.send(node_h.saturating_sub(lag)).await;
                }
            }
            ctx.sleep(Duration::from_millis(50)).await;
        }

        let seeded = victim_store
            .read()
            .map(|s| s.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH))
            .unwrap_or(false);
        seeded
    }

    /// The ordering clock seals `committee[2]`'s ceremony at `SEAL_DEADLINE` and
    /// finalizes within a couple of post-seal ticks — well before the epoch-2 boundary.
    /// The EL-finalized clock (lagged by `K`) reaches the seal deadline `K` ticks later,
    /// so by `SEAL_DEADLINE + 2` it has NOT yet memoized the share: the lag silently
    /// eats `K` blocks of the `DKG_MARGIN_BLOCKS` window. This is the wedge (Problem A):
    /// feeding the actor an `ordering−K` clock against an ordering-unit seal deadline.
    #[test]
    fn ordering_clock_seeds_within_margin_lagged_clock_slips_k() {
        const { assert!(DKG_MARGIN_BLOCKS >= K, "test geometry assumes MARGIN >= K") };
        let assert_by = SEAL_DEADLINE + 2;
        const { assert!(SEAL_DEADLINE + 2 < BOUNDARY, "assert point must precede the boundary") };

        let runtime = deterministic::Runner::default();
        let ordering_seeded =
            runtime.start(|ctx| async move { dkg_seeded_by(ctx, 0, assert_by).await });
        assert!(
            ordering_seeded,
            "ordering clock must memoize (PK_2, share) by SEAL_DEADLINE+2 (within margin)"
        );

        let runtime = deterministic::Runner::default();
        let lagged_seeded =
            runtime.start(|ctx| async move { dkg_seeded_by(ctx, K, assert_by).await });
        assert!(
            !lagged_seeded,
            "EL-finalized (ordering−K) clock must NOT have memoized the share yet — \
             the K-lag eats the margin (reproduces the wedge)"
        );
    }

    /// Regression for the boundary-stall deadlock. Feed heights only up to the seal
    /// deadline — every dealer seals and broadcasts its `Reveal` — then FREEZE the
    /// feed (no further `on_height` ticks, modelling reth-finalized frozen at the
    /// unfinalizable boundary block) while the sim keeps delivering those Reveals.
    /// The victim must STILL memoize `(PK_2, share)`, driven by `on_message` →
    /// `drive_finalization`. PRE-fix (finalize only inside `on_height`) the frozen
    /// feed starves it and it never seeds; POST-fix the Reveal event finalizes it.
    #[test]
    fn frozen_feed_seeds_via_reveal_event() {
        // The ceremony ENTERS at epoch_start(1) and SEALS on the next tick (the seal
        // step runs before `maybe_start` inserts it, so the seal lands one tick after
        // entry). Freeze right after that seal tick — every dealer has sealed and
        // broadcast its Reveal — then tick on with the feed frozen so the Reveals are
        // delivered purely over the network: finalize must then come from `on_message`,
        // not `on_height` (which is frozen). This is the boundary-stall the fix targets.
        const SEAL_TICK: u64 = SEAL_DEADLINE + 1;
        const TAIL: u64 = 6;
        const { assert!(SEAL_TICK + TAIL < BOUNDARY, "stay within the margin window") };
        let runtime = deterministic::Runner::default();
        let seeded = runtime.start(|ctx| async move {
            dkg_seeded_with_freeze(ctx, 0, SEAL_TICK + TAIL, SEAL_TICK, INTERVAL, 0, 0).await
        });
        assert!(
            seeded,
            "share must finalize via the Reveal event with the height feed frozen at \
             the seal deadline — the boundary-stall deadlock fix"
        );
    }

    /// Start-race regression (Fix 1). With a real dealing window (interval 30, margin
    /// 10 ⇒ the ceremony enters at epoch_start(1)=30 and seals at the deadline 50, a
    /// 20-tick window), the first TWO nodes start 2 ticks LATE, so each receives the
    /// two EARLY dealers' `Commitment`+`Share` BEFORE its own `maybe_start`. PRE-fix
    /// those are dropped ⇒ the early dealers collect only 2 acks ⇒ seal
    /// `TooManyReveals` ⇒ `select` rejects 2 of 4 ⇒ `< quorum(3)` ⇒ `DkgFailed` forever
    /// (the docker wedge). POST-fix they are BUFFERED and drained on start, every dealer
    /// is acked, and the victim seeds.
    #[test]
    fn start_race_buffers_early_dealings() {
        const INTERVAL_LONG: u64 = 30;
        let runtime = deterministic::Runner::default();
        let seeded = runtime.start(|ctx| async move {
            dkg_seeded_with_freeze(ctx, 0, 56, 56, INTERVAL_LONG, 2, 2).await
        });
        assert!(
            seeded,
            "early peer dealings that race ahead of `maybe_start` must be buffered + \
             drained (not dropped) so every dealer is acked and the victim seeds"
        );
    }

    /// Leak regression (H2). A ceremony whose committee has a quorum it never reaches
    /// (here only the victim deals; its three peers never reveal) is SEALED at the
    /// deadline but `ready()` stays false, so `drive_finalization` never removes it.
    /// Once the clock crosses the epoch boundary the per-tick sweep must evict it from
    /// `ceremonies`/`sealed` — otherwise it lingers and re-probes `ready()` forever.
    /// Drives `on_height` directly (not `run`) to inspect the actor's internal state.
    #[test]
    fn stalled_ceremony_is_evicted_past_boundary() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let oracle: Oracle<PeerPubkey, SimContext> = {
                let (network, oracle) = Network::new(
                    ctx.with_label("sim_net"),
                    SimConfig {
                        max_size: 1024 * 1024,
                        disconnect_on_block: false,
                        tracked_peer_sets: NZUsize!(4),
                    },
                );
                network.start();
                oracle
            };
            let mut rng = StdRng::seed_from_u64(3);
            let keys: Vec<Ed25519PrivateKey> =
                (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
            // committee n=4 ⇒ quorum 3; only the victim runs ⇒ 1 valid log < quorum ⇒ stall.
            let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
            oracle.manager().track(0, committee.clone()).await;
            let me = keys[0].clone();
            let pk = me.public_key();
            let (sender, receiver) = oracle
                .control(pk.clone())
                .register(
                    fluentbase_p2p::constants::BEACON_CHANNEL,
                    fluentbase_p2p::constants::BEACON_QUOTA,
                )
                .await
                .expect("register BEACON_CHANNEL");
            let committee_for: CommitteeFor = {
                let set = committee.clone();
                Arc::new(move |_epoch: u64| Some(set.clone()))
            };
            let store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
            let mut actor = DkgActor::new(
                b"FLUENT_DPOS_V1_leaktest".to_vec(),
                me,
                sender,
                receiver,
                committee_for,
                store,
                Arc::new(tokio::sync::Notify::new()),
                ACTIVATION,
                INTERVAL,
                crate::beacon::metrics::BeaconMetrics::default(),
                None,
            );
            let mut arng = StdRng::seed_from_u64(9);

            // Through the seal deadline: committee[2] enters (height 10), seals (11),
            // then stalls (1 valid log < quorum 3, so `ready()` never holds).
            for h in 0..=(SEAL_DEADLINE + 2) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor.ceremonies.contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH)
                    && actor.sealed.contains(&DETERMINISTIC_BOOTSTRAP_EPOCH),
                "precondition: committee[2] must be sealed-but-stalled before the boundary"
            );

            // Cross the epoch-2 boundary: the sweep must evict the stalled entry.
            for h in (SEAL_DEADLINE + 3)..=(BOUNDARY + 1) {
                actor.on_height(h, &mut arng).await;
            }
            assert!(
                actor.ceremonies.is_empty(),
                "stalled ceremony must be evicted once its boundary passes"
            );
            assert!(
                actor.sealed.is_empty(),
                "stalled `sealed` entry must be evicted once its boundary passes"
            );
        });
    }
}
