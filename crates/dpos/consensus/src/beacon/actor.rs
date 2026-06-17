//! Networked live-DKG actor: wraps [`DkgCeremony`] and drives committee[E]'s
//! self-DKG over `BEACON_CHANNEL` during epoch E-1.
//!
//! Single-ceremony-per-epoch, NO Muxer: each `DkgMsg` carries its `ceremony_epoch`
//! and the actor drops any message not for an active ceremony (epoch-tag filter).
//! Ceremonies for E and E+1 are temporally disjoint (the collection window spans
//! ~all of E-1), so at most a couple are in flight.
//!
//! Lifecycle, driven by the finalized-height stream + chain committee reads:
//! - entering epoch E-1 (committee[E] != committee[E-1]) → `DkgCeremony::start`,
//!   broadcast commitment + send private shares;
//! - finalized height reaches `epoch_start(E) - DKG_MARGIN_BLOCKS` → `seal_dealings`
//!   (broadcast the signed log);
//! - entering epoch E → `DkgCeremony::finalize` → write `(PK_E, share)` into the
//!   per-epoch [`BeaconKeyStore`] (Phase 5 wires that store into the per-epoch
//!   consensus scheme + the `commitEpochBeaconKey` producer).
//!
//! The actor never finalizes over a locally-selected Q — finalize runs only at the
//! epoch boundary over the logs collected by then (option-A: <quorum valid logs →
//! the write is skipped and the beacon naturally stalls, not a crash).

use crate::beacon::{
    ceremony::{DkgCeremony, Outgoing, Target},
    dkg_msg::DkgMsg,
    wire::BeaconMessage,
};
use commonware_codec::{Encode as _, Read as _, ReadExt as _};
use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
use commonware_p2p::{Receiver, Recipients, Sender};
use commonware_utils::ordered::Set;
use fluentbase_bls::{scheme::BeaconKey, PeerPubkey};
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    sync::{Arc, RwLock},
};

/// Blocks of slack before the epoch-E boundary at which dealing collection closes
/// and dealers seal (broadcast their signed logs) — the echo-settle tail. Pinned
/// off the on-chain `epochBlockInterval`, not an absolute window (see Q4).
pub const DKG_MARGIN_BLOCKS: u64 = 10;

/// The per-epoch beacon material the live DKG produces. Phase 5 reads this store
/// to source each epoch's consensus-scheme `BeaconKey` + drive the on-chain commit.
pub type BeaconKeyStore = Arc<RwLock<BTreeMap<u64, BeaconKey>>>;

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
    keys: BeaconKeyStore,
    dpos_activation: u64,
    epoch_interval: u64,
    /// Active ceremonies keyed by their target epoch E.
    ceremonies: BTreeMap<u64, DkgCeremony>,
    /// Target epochs whose dealing phase has been sealed.
    sealed: BTreeSet<u64>,
    /// Highest epoch we have already reacted to (for boundary detection).
    last_epoch: u64,
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
        keys: BeaconKeyStore,
        dpos_activation: u64,
        epoch_interval: u64,
    ) -> Self {
        Self {
            namespace,
            me_key,
            sender,
            receiver,
            committee_for,
            keys,
            dpos_activation,
            epoch_interval,
            ceremonies: BTreeMap::new(),
            sealed: BTreeSet::new(),
            last_epoch: 0,
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
        // Seal any active ceremony whose collection deadline has passed.
        let mut to_send: Vec<Outgoing> = Vec::new();
        let due: Vec<u64> = self
            .ceremonies
            .keys()
            .copied()
            .filter(|e| !self.sealed.contains(e) && height >= self.epoch_start(*e).saturating_sub(DKG_MARGIN_BLOCKS))
            .collect();
        for e in due {
            if let Some(c) = self.ceremonies.get_mut(&e) {
                to_send.extend(c.seal_dealings());
                self.sealed.insert(e);
            }
        }

        // Detect epoch transitions (a height may cross more than one boundary).
        let now = self.epoch_of(height);
        while self.last_epoch < now {
            let entered = self.last_epoch + 1;
            // Finalize the ceremony that targeted the just-entered epoch.
            if let Some(c) = self.ceremonies.remove(&entered) {
                match c.finalize(rng) {
                    Ok((out, share)) => {
                        let namespace = self.namespace.clone();
                        let key: BeaconKey = (out.public().clone(), Some(share), namespace);
                        if let Ok(mut store) = self.keys.write() {
                            store.insert(entered, key);
                        }
                        tracing::info!(epoch = entered, "live DKG: PK_epoch finalized + stored");
                    }
                    Err(e) => tracing::warn!(epoch = entered, ?e, "live DKG: finalize failed (under-quorum) — beacon stalls for this epoch"),
                }
            }
            self.sealed.remove(&entered);
            // Start the ceremony for the NEXT epoch if the committee changes.
            self.maybe_start(entered + 1, rng, &mut to_send);
            self.last_epoch = entered;
        }

        self.broadcast_all(to_send).await;
    }

    /// Start a ceremony for `target` (run during the just-entered epoch) when the
    /// committee actually changes; an unchanged committee carries the key forward
    /// (no ceremony — Phase 5 reuses the prior epoch's `BeaconKey`).
    fn maybe_start(&mut self, target: u64, rng: &mut impl CryptoRngCore, out: &mut Vec<Outgoing>) {
        if target == 0 || self.ceremonies.contains_key(&target) {
            return;
        }
        let (Some(cur), Some(next)) = ((self.committee_for)(target - 1), (self.committee_for)(target))
        else {
            return;
        };
        if next == cur {
            return; // carry-forward
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
