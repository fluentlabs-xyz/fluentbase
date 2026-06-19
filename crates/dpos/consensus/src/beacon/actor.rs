//! Networked live-DKG actor: wraps [`DkgCeremony`] and drives committee[E]'s
//! self-DKG over `BEACON_CHANNEL` during epoch E-1.
//!
//! Single-ceremony-per-epoch, NO Muxer: each `DkgMsg` carries its `ceremony_epoch`
//! and the actor drops any message not for an active ceremony (epoch-tag filter).
//! Ceremonies for E and E+1 are temporally disjoint (the collection window spans
//! ~all of E-1), so at most a couple are in flight.
//!
//! Lifecycle, driven by the finalized-height stream + chain committee reads:
//! - entering epoch E-1 (committee[E] != committee[E-1] AND this node ∈
//!   committee[E]) → `DkgCeremony::start`, broadcast commitment + send shares;
//! - finalized height reaches `epoch_start(E) - DKG_MARGIN_BLOCKS` → `seal_dealings`
//!   (broadcast the signed log);
//! - once a sealed ceremony has a selectable quorum (`DkgCeremony::ready`, probed
//!   each subsequent tick — STILL DURING the margin window, BEFORE the epoch-E
//!   boundary block is proposed/verified) → `DkgCeremony::finalize` → memoize
//!   `(PK_E, share)` into the per-epoch [`CeremonyStore`]. The consensus verify
//!   path reads the share for the C share-on-polynomial gate, `propose` reads
//!   `PK_E` for the boundary `beacon_outcome`, and Phase 5's finalized-boundary
//!   swap reads both for the per-epoch signing slot + `commitEpochBeaconKey`.
//!
//! The actor never finalizes over a locally-selected Q before sealing, and never
//! over an under-quorum log set (`ready` gates it). <quorum valid logs → no store
//! entry → the beacon naturally stalls for that epoch (option A), not a crash.

use crate::beacon::{
    ceremony::{CeremonyOutput, DkgCeremony, Outgoing, Target},
    dkg_msg::DkgMsg,
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
    dpos_activation: u64,
    epoch_interval: u64,
    metrics: crate::beacon::metrics::BeaconMetrics,
    /// Directory for on-disk share persistence (item A): the parent of
    /// `--dpos.beacon-share-path`. `None` ⇒ no share-path configured (e.g. a
    /// case-(b) sharing-only node) ⇒ memoized shares stay in-memory only.
    share_dir: Option<PathBuf>,
    /// Active ceremonies keyed by their target epoch E.
    ceremonies: BTreeMap<u64, DkgCeremony>,
    /// Target epochs whose dealing phase has been sealed.
    sealed: BTreeSet<u64>,
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
            dpos_activation,
            epoch_interval,
            metrics,
            share_dir,
            ceremonies: BTreeMap::new(),
            sealed: BTreeSet::new(),
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
                    Ok((from, buf)) => self.on_message(from, buf.as_ref()).await,
                    Err(_) => break,
                },
            }
        }
    }

    async fn on_height(&mut self, height: u64, rng: &mut impl CryptoRngCore) {
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

        // 2. Compute + memoize any SEALED ceremony that now has a selectable quorum.
        //    This runs STILL DURING the margin window — before the epoch's boundary
        //    block is proposed/verified — so the verify-path C gate can read the
        //    share. `ready` probes non-destructively (Logs clone); `finalize` then
        //    consumes the fulfilled ceremony.
        //
        //    Item F (PK_E byte-identity — SAFETY holds, residual is LIVENESS only):
        //    `finalize` is deterministic over a given selected log set, so all honest
        //    members that select the same logs derive the byte-identical `PK_E` (the
        //    n=51 `dkg::seed_is_threshold_unique_at_n51` test confirms the seed at the
        //    production committee size). If honest members select DIFFERENT log sets
        //    (no consensus-pinned QUAL — out of scope, `dpos_beacon_share_reshare`),
        //    a minority `PK_E` fails the boundary "C" share-on-poly gate
        //    (`application::beacon_gate_decision`) → no qualifying quorum → view-change;
        //    and a forged/divergent `PK_E` that threaded the C gate still cannot
        //    finalize, because the Stage-2 certify hook (`beacon::certify`) σ-verifies
        //    the recovered seed against the asserted `PK_E` and Nullifies on mismatch.
        //    So a divergent log set NEVER yields a forged finalized seed — the only
        //    residual is a LIVENESS stall of that epoch's beacon (option-A halt), paired
        //    with the `dkg_ceremony_fail_total` metric below.
        let ready: Vec<u64> = self
            .ceremonies
            .iter()
            .filter(|(e, c)| self.sealed.contains(*e) && c.ready(rng))
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

        // 3. Start the NEXT epoch's DKG ceremony. Retried on EVERY tick (not just
        //    once at the epoch transition): committee[E+1] is committed on-chain
        //    sometime DURING epoch E, which can land AFTER the actor (driven by
        //    lagging finalized heights) first enters E — a single-shot check at the
        //    transition would see the committee still unchanged, carry forward, and
        //    NEVER deal, so the E+1 boundary block wedges (no PK_{E+1}). maybe_start
        //    is idempotent (no-op once the ceremony is in flight, already computed,
        //    the committee is unchanged, or this node is not a member), so retrying
        //    until committee[E+1] is visible+changed is safe.
        let now = self.epoch_of(height);
        self.maybe_start(now + 1, rng, &mut to_send);

        self.broadcast_all(to_send).await;
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
        if next == cur {
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
                tracing::info!(epoch = target, "live DKG: ceremony started");
            }
            Err(e) => tracing::warn!(epoch = target, ?e, "live DKG: ceremony start failed"),
        }
    }

    async fn on_message(&mut self, from: PeerPubkey, buf: &[u8]) {
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
        // Epoch-tag filter: only an active ceremony for this epoch processes it.
        if let Some(c) = self.ceremonies.get_mut(&msg.ceremony_epoch) {
            let out = c.handle(from, msg.body);
            self.broadcast_all(out).await;
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
