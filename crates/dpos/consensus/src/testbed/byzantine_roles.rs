//! The stand's byzantine wrappers for [`Role::TwoReveals`](super::stand::Role) and
//! [`Role::ForgedSeedUpstream`](super::stand::Role) — Э3.3, the register entries
//! R-002 and R-008.
//!
//! Both are WRAPPERS around a seam the honest node already has, and neither is
//! reachable without the `dpos-devnet-byzantine` feature. Nothing in production
//! changes: a wrapper sits between the plane and the wire, so the code that has
//! to notice the tampering is the same code a production node runs.
//!
//! * [`TwoRevealSender`] wraps the `BEACON_CHANNEL` sender half that
//!   `build_node` hands `BeaconConfig::beacon_channel`. It intercepts the ONE
//!   `DkgBody::Reveal` its node broadcasts at the seal deadline
//!   (`ceremony.rs::seal_dealings`) and sends the VICTIM a second, independently
//!   dealt and validly signed log of the same dealer over the same `Info`, while
//!   every other member receives the original. The second log cannot be
//!   re-signed from the wire bytes — `SignedDealerLog::sign` is private
//!   (`CW:cryptography/src/bls12381/dkg.rs:1203`) — so the wrapper runs a SECOND
//!   `Dealer::start::<N3f1>` over the same `Info` and `finalize`s it
//!   (`CW:…dkg.rs:1477`, `:1548`). That second dealer collected no acks, so its
//!   log carries `DealerResult::TooManyReveals` (reveals `n` > `max_reveals` =
//!   f); it is still a `check`-valid log of that dealer, which is all
//!   `DkgCeremony::record_checked_log` looks at.
//! * [`WithholdingRandomness`] wraps the plane's `Randomness` and rebuilds the
//!   signer scheme over the epoch's VERIFY-ONLY oracle (`oracle_for`, whose
//!   `BeaconOracle::me` is `None`). By `oracle.rs:172-189` such an oracle
//!   answers `None` to `sign_partial`, and by `combined_scheme.rs:284-287` a
//!   `sign` whose `sign_partial` answers `None` casts NO VOTE. That is exactly
//!   "this dealer withholds its seed partial", expressed at the one place the
//!   partial is produced.
//! * [`ForgedSeedProducer`] wraps the frontier plane's `Producer` half
//!   (`stand::frontier_plane`, today `fakes::CountingHandler`) and swaps the σ
//!   slot of the `Finalized{h}` certificates it serves inside a height window,
//!   leaving the multisig half byte-identical. The σ it plants is a REAL σ of
//!   another round of the same epoch, harvested from an earlier answer, so it is
//!   a valid G1 point that cannot verify for the round it is planted into.
//!
//! Every wrapper records what it did into a [`ByzReport`], and the tests assert
//! the tampering took effect BEFORE they assert anything about the node's
//! reaction — a green branch over a wrapper that swapped nothing would state
//! nothing at all.

use super::fakes::{ByzReport, CountingHandler};
use crate::{
    beacon::{
        ceremony::info_for,
        dkg_msg::{DealerReveal, DkgBody, DkgMsg},
        keys::InvalidSeed,
        seed::Seed,
        surface::{PinEffort, Randomness, ShareProbe, SignerVerdict},
        verified_seed::VerifiedSeed,
        wire::BeaconMessage,
    },
    digest::Digest,
    order_block::OrderBlock,
    plane_upstream::FrontierKey,
};
use alloy_primitives::keccak256;
use bytes::Bytes;
use commonware_codec::{Decode as _, Encode as _, Read as _, ReadExt as _};
use commonware_consensus::{
    simplex::types::Finalization,
    types::{Epoch, Round},
};
use commonware_cryptography::{
    bls12381::{dkg::Dealer, primitives::variant::MinSig},
    ed25519::PrivateKey as Ed25519PrivateKey,
    Signer as _,
};
use commonware_p2p::{CheckedSender, LimitedSender, Recipients, Sender};
use commonware_resolver::{p2p::Producer, Consumer};
use commonware_runtime::IoBufs;
use commonware_utils::{channel::oneshot as cw_oneshot, ordered::Set, N3f1};
use fluentbase_bls::{
    combined_scheme::CombinedCertificate, keys::ValidatorBlsKeypair, oracle::SeedOracle,
    BlsSignature, PeerPubkey, Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use rand_08::rngs::StdRng;
use rand_core::SeedableRng as _;
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tokio::sync::Notify;

/// The seed of the SECOND dealer a [`TwoRevealSender`] runs. Fixed, so the whole
/// role is deterministic under the stand's deterministic runner, and different
/// from the honest dealer's key-derived seed (`ceremony.rs::dealer_seed_rng`), so
/// the two polynomials — hence the two logs — differ.
const SECOND_DEALER_SEED: u64 = 0x5EC0_0DEA_1E12_5EED;

/// The `(Cert, OrderBlock)` pair the frontier wire carries, spelled out here
/// because `plane_upstream`'s alias is private.
type FrontierPair = (Finalization<BlsScheme, Digest>, OrderBlock);

// ---------------------------------------------------------------------------
// R-002 — the two-log dealer
// ---------------------------------------------------------------------------

/// Everything the wrapper needs to mint the second log, plus who to send it to.
#[derive(Clone)]
pub(super) struct TwoRevealCfg {
    /// This node's p2p key — the second `Dealer`'s identity, so both logs are
    /// signed by the SAME dealer.
    pub me_key: Ed25519PrivateKey,
    /// The victim of the split (node A of the register entry).
    pub victim: PeerPubkey,
    /// Every other member: they receive the original log.
    pub others: Vec<PeerPubkey>,
    /// `committee[epoch]`, read exactly as the plane reads it.
    pub committee_for: Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>,
    /// The DKG namespace the actor signs its logs under — `plane.rs:506`'s
    /// `seed_namespace(fluent_namespace(chain_id))`.
    pub namespace: Vec<u8>,
    pub report: ByzReport,
}

/// The `BEACON_CHANNEL` sender half of a two-log dealer.
///
/// `inner` is the simulated network's own sender; `cfg` is `None` on every
/// honest node, and then this type is a pure pass-through (the stand keeps ONE
/// sender type across roles so `beacon::build`'s `Se` does not depend on the
/// role).
#[derive(Clone)]
pub(super) struct TwoRevealSender<S> {
    inner: S,
    cfg: Option<TwoRevealCfg>,
}

impl<S> TwoRevealSender<S> {
    pub(super) fn new(inner: S, cfg: Option<TwoRevealCfg>) -> Self {
        Self { inner, cfg }
    }
}

/// Mint a second, independently dealt but validly signed log of `me_key` over the
/// SAME `Info` the honest ceremony used. Returns `None` when the committee is not
/// readable (a transient read race) — the caller then forwards the original
/// untouched, and its `reveals_swapped` counter stays put, so a test can tell the
/// two apart.
fn second_log(cfg: &TwoRevealCfg, epoch: u64) -> Option<DealerReveal> {
    let committee = (cfg.committee_for)(epoch)?;
    let info = info_for(&cfg.namespace, epoch, committee).ok()?;
    let rng = StdRng::seed_from_u64(SECOND_DEALER_SEED ^ epoch);
    let (dealer, _pub_msg, _priv_msgs) =
        Dealer::<MinSig, Ed25519PrivateKey>::start::<N3f1>(rng, info, cfg.me_key.clone(), None)
            .ok()?;
    // No ack is ever fed to this dealer, so `finalize` writes
    // `DealerResult::TooManyReveals` (`CW:…dkg.rs:1548-1560`). The log is still
    // signed by this dealer over this `Info`, which is the whole of what
    // `DkgCeremony::record_checked_log` requires.
    Some(dealer.finalize::<N3f1>())
}

/// Split one intercepted `Reveal` into `(bytes for the others, bytes for the
/// victim)`, recording the tamper's own witness. `None` ⇒ nothing was swapped.
fn split_reveal(cfg: &TwoRevealCfg, wire: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
        .expect("MAX_COMMITTEE_SIZE > 0");
    let mut buf = wire;
    let BeaconMessage::Dkg(payload) = BeaconMessage::read(&mut buf).ok()?;
    let msg = DkgMsg::read_cfg(&mut payload.as_ref(), &max).ok()?;
    let DkgBody::Reveal(log1) = msg.body else {
        return None;
    };
    let epoch = msg.ceremony_epoch;
    cfg.report.with(|f| f.reveals_seen += 1);
    let log2 = second_log(cfg, epoch)?;

    // The tamper's self-check, run BEFORE the swap is allowed to happen: both
    // logs must pass the receiver's own predicate, and they must differ.
    let me = cfg.me_key.public_key();
    let committee = (cfg.committee_for)(epoch)?;
    let info = info_for(&cfg.namespace, epoch, committee).ok()?;
    let checks = |l: &DealerReveal| l.clone().check(&info).is_some_and(|(pk, _)| pk == me);
    let h1 = keccak256(log1.encode());
    let h2 = keccak256(log2.encode());
    assert_ne!(
        h1, h2,
        "the two-reveal role minted a SECOND log identical to the first — nothing is split"
    );
    let both_check = checks(&log1) && checks(&log2);
    assert!(
        both_check,
        "the two-reveal role minted a log the receiver's own `check` would drop"
    );

    let for_others = BeaconMessage::Dkg(
        DkgMsg {
            ceremony_epoch: epoch,
            body: DkgBody::Reveal(log1),
        }
        .encode(),
    )
    .encode()
    .to_vec();
    let for_victim = BeaconMessage::Dkg(
        DkgMsg {
            ceremony_epoch: epoch,
            body: DkgBody::Reveal(Box::new(log2)),
        }
        .encode(),
    )
    .encode()
    .to_vec();
    cfg.report.with(|f| {
        f.reveals_swapped += 1;
        f.log1_hash = Some(h1);
        f.log2_hash = Some(h2);
        f.both_logs_check = both_check;
        f.victim = Some(cfg.victim.clone());
    });
    Some((for_others, for_victim))
}

/// The `CheckedSender` half. The tamper lives HERE and not in a `Sender::send`
/// override, because `commonware_p2p` supplies `Sender` through a BLANKET impl
/// over `LimitedSender` (`CW:p2p/src/lib.rs:156-193`): the only seam a wrapper
/// owns is `check` → `CheckedSender::send`.
pub(super) struct TwoRevealChecked<S> {
    inner: S,
    recipients: Recipients<PeerPubkey>,
    cfg: Option<TwoRevealCfg>,
}

/// The wrapper's own send error. The inner error type is generic and cannot be
/// named across the `Checked<'a>` lifetime, and every caller of this channel
/// discards it (`actor.rs:2445` is `let _ = …send(…)`), so it is flattened to its
/// `Debug` text rather than threaded.
#[derive(Debug, thiserror::Error)]
#[error("two-reveal beacon sender: {0}")]
pub(super) struct SplitSendError(String);

impl<S: Sender<PublicKey = PeerPubkey>> CheckedSender for TwoRevealChecked<S> {
    type PublicKey = PeerPubkey;
    type Error = SplitSendError;

    async fn send(
        mut self,
        message: impl Into<IoBufs> + Send,
        priority: bool,
    ) -> Result<Vec<Self::PublicKey>, Self::Error> {
        let bytes: Bytes = Into::<IoBufs>::into(message).coalesce().into();
        let split = self
            .cfg
            .as_ref()
            .filter(|_| matches!(self.recipients, Recipients::All))
            .and_then(|cfg| split_reveal(cfg, bytes.as_ref()).map(|s| (cfg.clone(), s)));
        let Some((cfg, (for_others, for_victim))) = split else {
            return self
                .inner
                .send(self.recipients, bytes, priority)
                .await
                .map_err(|e| SplitSendError(format!("{e:?}")));
        };
        let mut sent = self
            .inner
            .send(Recipients::Some(cfg.others.clone()), for_others, priority)
            .await
            .map_err(|e| SplitSendError(format!("{e:?}")))?;
        sent.extend(
            self.inner
                .send(Recipients::One(cfg.victim.clone()), for_victim, priority)
                .await
                .map_err(|e| SplitSendError(format!("{e:?}")))?,
        );
        Ok(sent)
    }
}

impl<S: Sender<PublicKey = PeerPubkey>> LimitedSender for TwoRevealSender<S> {
    type PublicKey = PeerPubkey;
    type Checked<'a>
        = TwoRevealChecked<S>
    where
        Self: 'a;

    /// The rate check is the inner sender's own, re-run inside
    /// [`TwoRevealChecked::send`]: this stand's channels are registered at
    /// `Quota::per_second(NonZeroU32::MAX)`, so no recipient is ever filtered and
    /// admitting here changes nothing about what goes on the wire.
    async fn check(
        &mut self,
        recipients: Recipients<Self::PublicKey>,
    ) -> Result<Self::Checked<'_>, SystemTime> {
        Ok(TwoRevealChecked {
            inner: self.inner.clone(),
            recipients,
            cfg: self.cfg.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// R-002, second link — the dealer that withholds its seed partial
// ---------------------------------------------------------------------------

/// The plane's `Randomness` with ONE operation changed: the signer scheme is
/// rebuilt over the epoch's verify-only oracle, so this node produces no seed
/// partial and therefore (`combined_scheme.rs:284-287`) casts no vote.
pub(super) struct WithholdingRandomness {
    inner: Arc<dyn Randomness>,
    chain_id: u64,
    report: ByzReport,
}

impl WithholdingRandomness {
    pub(super) fn wrap(
        inner: Arc<dyn Randomness>,
        chain_id: u64,
        report: ByzReport,
    ) -> Arc<dyn Randomness> {
        Arc::new(Self {
            inner,
            chain_id,
            report,
        })
    }
}

impl Randomness for WithholdingRandomness {
    fn record_seed(&self, verified: VerifiedSeed) {
        self.inner.record_seed(verified)
    }
    fn quarantine_seed(&self, round: Round, seed: BlsSignature) {
        self.inner.quarantine_seed(round, seed)
    }
    fn on_invalid_seed(&self, epoch: u64) -> InvalidSeed {
        self.inner.on_invalid_seed(epoch)
    }
    fn seed_for(&self, round: Round) -> Option<Seed> {
        self.inner.seed_for(round)
    }
    fn terminal_seed_at(&self, round: Round) -> Option<Seed> {
        self.inner.terminal_seed_at(round)
    }
    fn seed_edge(&self) -> Arc<Notify> {
        self.inner.seed_edge()
    }
    fn mandatory_at(&self, epoch: u64) -> bool {
        self.inner.mandatory_at(epoch)
    }
    fn share_probe(&self, epoch: Epoch) -> ShareProbe {
        self.inner.share_probe(epoch)
    }
    fn participation_edge(&self) -> Arc<Notify> {
        self.inner.participation_edge()
    }
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        self.inner.oracle_for(epoch)
    }
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
        self.inner.ensure_key(epoch, effort)
    }
    fn key_edge(&self) -> Arc<Notify> {
        self.inner.key_edge()
    }
    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch) {
        self.inner.observe_epoch(reconciled, entered_frontier)
    }
    fn observe_cert(&self, epoch: u64) {
        self.inner.observe_cert(epoch)
    }

    /// The inner verdict is taken FIRST — it carries the plane's own side effects
    /// (the gates, the counters, the key publication `signer_scheme`'s doc calls
    /// a data dependency) — and only a `Signs` arm is rebuilt.
    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        let honest = match self.inner.signer_scheme(epoch, snap, keypair) {
            SignerVerdict::Signs(scheme) => scheme,
            other => return other,
        };
        let Some(oracle) = self.inner.oracle_for(epoch.get()) else {
            // A pre-beacon epoch: there is no partial to withhold.
            return SignerVerdict::Signs(honest);
        };
        let committee = match crate::scheme::epoch_committee_from_snapshot(snap) {
            Ok(c) => c,
            Err(e) => return SignerVerdict::InvalidCommittee(e),
        };
        let namespace = fluentbase_bls::fluent_namespace(self.chain_id);
        let Some(withheld) = fluentbase_bls::scheme::build_signer(
            &namespace,
            committee.bimap,
            keypair,
            epoch.get(),
            Some(oracle),
        ) else {
            return SignerVerdict::Signs(honest);
        };
        // The withholding's own witness: the honest scheme signs a probe subject
        // of this epoch and the rebuilt one does not. A `(false, false)` would
        // mean the node had no partial to withhold in the first place, and the
        // test asserting the halt would be asserting nothing.
        let probe = {
            use commonware_consensus::{simplex::types::Subject, types::View};
            use commonware_cryptography::certificate::Scheme as _;
            let round = Round::new(epoch, View::new(1));
            let honest_signs = {
                let subject: Subject<'_, Digest> = Subject::Nullify { round };
                honest.sign(subject).is_some()
            };
            let withheld_signs = {
                let subject: Subject<'_, Digest> = Subject::Nullify { round };
                withheld.sign(subject).is_some()
            };
            (honest_signs, withheld_signs)
        };
        self.report.with(|f| {
            f.schemes_withheld += 1;
            f.withhold_probe = Some(probe);
        });
        SignerVerdict::Signs(withheld)
    }
}

// ---------------------------------------------------------------------------
// R-008 — the upstream that serves a forged σ slot
// ---------------------------------------------------------------------------

/// The heights whose served certificates get a forged σ slot. Chosen to cover
/// the first block of the bootstrap epoch (`2 * epoch_len = 64`) and the six
/// after it — the window a follower with no `PK_2` has to walk.
pub(super) const FORGE_WINDOW: std::ops::RangeInclusive<u64> = 64..=70;

#[derive(Default)]
struct ForgeState {
    /// A real σ of some other round of the same epoch, harvested from an earlier
    /// answer: a valid G1 point that cannot verify for the round it is planted
    /// into. `(round the σ belongs to, σ)`.
    harvested: Option<(Round, BlsSignature)>,
    /// Every `(σ, the round it was served under)` this node has served — the
    /// archive-poisoning witness of a node that is NOT forging.
    served: std::collections::HashMap<Vec<u8>, Round>,
}

/// The frontier plane's `Producer` + `Consumer` with a forging `produce`.
#[derive(Clone)]
pub(super) struct ForgedSeedProducer {
    inner: CountingHandler,
    state: Arc<Mutex<ForgeState>>,
    report: ByzReport,
    active: bool,
}

impl ForgedSeedProducer {
    pub(super) fn new(inner: CountingHandler, report: ByzReport, active: bool) -> Self {
        Self {
            inner,
            state: Arc::new(Mutex::new(ForgeState::default())),
            report,
            active,
        }
    }

    /// Re-serve `bytes` with the σ slot replaced, or `None` when there is nothing
    /// to replace it with yet. Runs the tamper's own self-check on the result.
    fn forge(&self, height: u64, bytes: &Bytes) -> Option<Vec<u8>> {
        let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
        let (fin, block) = FrontierPair::decode_cfg(bytes.as_ref(), &(cap, ())).ok()?;
        let round = fin.proposal.round;
        let original = fin.certificate.seed()?;
        self.report.with(|f| f.certs_seen += 1);
        let plant = {
            let mut st = self.state.lock().expect("forge state");
            match st.harvested {
                // Harvest first: the σ this answer carries becomes the payload of
                // the NEXT round's forgery.
                None => {
                    st.harvested = Some((round, original));
                    None
                }
                Some((r, _)) if r == round => None,
                Some((_, sigma)) => Some(sigma),
            }
        }?;
        if plant == original {
            return None;
        }
        let forged = Finalization::<BlsScheme, Digest> {
            proposal: fin.proposal.clone(),
            certificate: CombinedCertificate {
                vote: fin.certificate.vote.clone(),
                seed: Some(plant),
            },
        };
        let wire = (forged, block).encode().to_vec();

        // Self-check on the BYTES that go out, not on the struct we built: decode
        // them back the way the victim's `decode_frontier` will.
        let (back, _) = FrontierPair::decode_cfg(wire.as_slice(), &(cap, ()))
            .expect("the forged pair must decode");
        let seed_back = back.certificate.seed();
        let vote_intact = back.certificate.vote.encode() == fin.certificate.vote.encode();
        assert_eq!(
            seed_back,
            Some(plant),
            "the forged σ slot did not read back as the planted value"
        );
        assert_ne!(
            seed_back,
            Some(original),
            "the forged σ slot read back as the ORIGINAL σ — nothing was swapped"
        );
        assert!(
            vote_intact,
            "the forge touched the multisig half of the certificate"
        );
        self.report.with(|f| {
            f.certs_forged += 1;
            f.forged_heights.push(height);
            f.forged_seed_differs = f.certs_forged == 1 || f.forged_seed_differs;
            f.forged_vote_half_intact = f.certs_forged == 1 || f.forged_vote_half_intact;
            f.forged_seed_differs &= seed_back != Some(original);
            f.forged_vote_half_intact &= vote_intact;
        });
        Some(wire)
    }
}

impl ForgedSeedProducer {
    /// The archive-poisoning witness. A σ is unique per `(round, PK)`
    /// (`beacon/seed.rs`), so serving the SAME σ under two different rounds is
    /// something an honest archive cannot do: it means this node stored a
    /// certificate whose σ slot had been swapped and is now relaying it.
    fn note_served(&self, height: u64, bytes: &Bytes) {
        let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
        let Ok((fin, _block)) = FrontierPair::decode_cfg(bytes.as_ref(), &(cap, ())) else {
            return;
        };
        let round = fin.proposal.round;
        let Some(sigma) = fin.certificate.seed() else {
            return;
        };
        let replayed = {
            let mut st = self.state.lock().expect("forge state");
            matches!(st.served.insert(sigma.encode().to_vec(), round), Some(seen) if seen != round)
        };
        if replayed {
            self.report.with(|f| f.served_seed_replays.push(height));
        }
    }
}

impl Producer for ForgedSeedProducer {
    type Key = FrontierKey;

    async fn produce(&mut self, key: FrontierKey) -> cw_oneshot::Receiver<Bytes> {
        let rx = self.inner.produce(key).await;
        let FrontierKey::Finalized { height } = key else {
            return rx;
        };
        if !self.active {
            // Not forging: WATCH what this node serves instead. That is the only
            // way the stand can see a follower relaying a forgery it accepted.
            let Ok(bytes) = rx.await else {
                let (tx, rx) = cw_oneshot::channel::<Bytes>();
                drop(tx);
                return rx;
            };
            self.note_served(height, &bytes);
            let (tx, rx) = cw_oneshot::channel::<Bytes>();
            let _ = tx.send(bytes);
            return rx;
        }
        if !FORGE_WINDOW.contains(&height) {
            return rx;
        }
        // `FrontierHandler::produce` fills the channel INLINE before returning it
        // (`plane_upstream.rs`), so this await never parks; an archive miss shows
        // up as a dropped sender, which is served on as "no data".
        let Ok(bytes) = rx.await else {
            let (tx, rx) = cw_oneshot::channel::<Bytes>();
            drop(tx);
            return rx;
        };
        let out = self.forge(height, &bytes).map(Bytes::from).unwrap_or(bytes);
        let (tx, rx) = cw_oneshot::channel::<Bytes>();
        let _ = tx.send(out);
        rx
    }
}

impl Consumer for ForgedSeedProducer {
    type Key = FrontierKey;
    type Value = Bytes;
    type Failure = ();

    async fn deliver(&mut self, key: FrontierKey, value: Bytes) -> bool {
        self.inner.deliver(key, value).await
    }

    async fn failed(&mut self, key: FrontierKey, failure: ()) {
        self.inner.failed(key, failure).await
    }
}
