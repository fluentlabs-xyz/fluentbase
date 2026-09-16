//! The stand's byzantine wrappers for the frontier-serve and DKG roles.
//!
//! All are wrappers around a seam the honest node already has, and none is
//! reachable without the `dpos-devnet-byzantine` feature, so the code that has to
//! notice the tampering is the code a production node runs.
//!
//! * [`TwoRevealSender`] wraps the `BEACON_CHANNEL` sender and sends the victim a
//!   second, independently dealt and validly signed log of the same dealer.
//! * [`WithholdingRandomness`] rebuilds the signer scheme over the epoch's
//!   verify-only oracle, so the node produces no seed partial and casts no vote.
//! * [`ForgedSeedProducer`] wraps the frontier plane's producer and applies the
//!   tamper [`ForgeMode`] selects.
//!
//! Every wrapper records what it did into a [`ByzReport`], and the tests assert
//! the tampering took effect before they assert anything about the node's
//! reaction.

use super::fakes::{ByzReport, CountingHandler, ElNetwork, FakeChain};
use crate::{
    beacon::{
        testing::{info_for, BeaconMessage, DealerReveal, DkgBody, DkgMsg},
        Beacon, BeaconEvent, DataFault, Observed, ObservedCertificate, PinEffort, Seed, ShareProbe,
        SignerVerdict,
    },
    cert_follow::UpstreamFinalized,
    cold_start_jump::verify_jump_structural,
    digest::Digest,
    order_block::OrderBlock,
    plane_upstream::FrontierKey,
};
use alloy_primitives::{keccak256, B256};
use bytes::Bytes;
use commonware_codec::{Decode as _, Encode as _, Read as _, ReadExt as _};
use commonware_consensus::{
    simplex::types::{Finalization, Proposal},
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
use tokio::sync::{broadcast, mpsc};

/// The seed of the second dealer a [`TwoRevealSender`] runs. Fixed, so the whole
/// role is deterministic under the stand's runner, and different from the honest
/// dealer's key-derived seed, so the two logs differ.
const SECOND_DEALER_SEED: u64 = 0x5EC0_0DEA_1E12_5EED;

/// The `(Cert, OrderBlock)` pair the frontier wire carries.
type FrontierPair = (Finalization<BlsScheme, Digest>, OrderBlock);

/// Everything the wrapper needs to mint the second log, plus who to send it to.
#[derive(Clone)]
pub(super) struct TwoRevealCfg {
    /// This node's p2p key — the second `Dealer`'s identity, so both logs are
    /// signed by the same dealer.
    pub me_key: Ed25519PrivateKey,
    /// The victim of the split.
    pub victim: PeerPubkey,
    /// Every other member: they receive the original log.
    pub others: Vec<PeerPubkey>,
    /// `committee[epoch]`, read exactly as the plane reads it.
    pub committee_for: Arc<dyn Fn(u64) -> Option<Set<PeerPubkey>> + Send + Sync>,
    /// The DKG namespace the actor signs its logs under.
    pub namespace: Vec<u8>,
    pub report: ByzReport,
}

/// The `BEACON_CHANNEL` sender half of a two-log dealer.
///
/// `cfg` is `None` on every honest node, and then this type is a pure pass-through
/// (the stand keeps one sender type across roles so `beacon::build`'s `Se` does not
/// depend on the role).
#[derive(Clone)]
pub(super) struct TwoRevealSender<S> {
    inner: S,
    report: ByzReport,
    cfg: Option<TwoRevealCfg>,
}

impl<S> TwoRevealSender<S> {
    pub(super) fn new(inner: S, report: ByzReport, cfg: Option<TwoRevealCfg>) -> Self {
        Self { inner, report, cfg }
    }
}

/// Record an outgoing `ShareConfirm`. Runs on every node — see
/// [`ByzFacts::confirms_sent`] for why the honest ones matter.
fn note_confirm(report: &ByzReport, wire: &[u8]) {
    let max = NonZeroU32::new(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32)
        .expect("MAX_COMMITTEE_SIZE > 0");
    let mut buf = wire;
    let Ok(BeaconMessage::Dkg(payload)) = BeaconMessage::read(&mut buf) else {
        return;
    };
    let Ok(msg) = DkgMsg::read_cfg(&mut payload.as_ref(), &max) else {
        return;
    };
    let DkgBody::Confirm(confirm) = msg.body else {
        return;
    };
    report.with(|f| {
        f.confirms_sent
            .push((msg.ceremony_epoch, confirm.idx, confirm.recorded.clone()))
    });
}

/// Mint a second, independently dealt but validly signed log of `me_key` over the
/// same `Info` the honest ceremony used. Returns `None` when the committee is not
/// readable, and the caller then forwards the original untouched.
fn second_log(cfg: &TwoRevealCfg, epoch: u64) -> Option<DealerReveal> {
    let committee = (cfg.committee_for)(epoch)?;
    let info = info_for(&cfg.namespace, epoch, committee).ok()?;
    let rng = StdRng::seed_from_u64(SECOND_DEALER_SEED ^ epoch);
    let (dealer, _pub_msg, _priv_msgs) =
        Dealer::<MinSig, Ed25519PrivateKey>::start::<N3f1>(rng, info, cfg.me_key.clone(), None)
            .ok()?;
    // No ack is ever fed to this dealer, so `finalize` writes
    // `DealerResult::TooManyReveals`. The log is still signed by this dealer over
    // this `Info`, which is what `DkgCeremony::record_checked_log` requires.
    Some(dealer.finalize::<N3f1>())
}

/// Split one intercepted `Reveal` into the bytes for the others and the bytes for
/// the victim. `None` means nothing was swapped.
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

    // The tamper's self-check, run before the swap is allowed: both logs must pass
    // the receiver's own predicate, and they must differ.
    let me = cfg.me_key.public_key();
    let committee = (cfg.committee_for)(epoch)?;
    let info = info_for(&cfg.namespace, epoch, committee).ok()?;
    let checks = |l: &DealerReveal| l.clone().check(&info).is_some_and(|(pk, _)| pk == me);
    // A `Reveal` this node did not sign is not ours to split: the actor broadcasts
    // only its own sealed log, and relaying somebody else's would forge a third
    // party's equivocation.
    if !checks(&log1) {
        return None;
    }
    let h1 = keccak256(log1.encode());
    let h2 = keccak256(log2.encode());
    assert_ne!(
        h1, h2,
        "the two-reveal role minted a SECOND log identical to the first — nothing is split"
    );
    let both_check = checks(&log2);
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

/// The `CheckedSender` half. The tamper lives here rather than in a `Sender::send`
/// override, because `commonware_p2p` supplies `Sender` through a blanket impl over
/// `LimitedSender`: the only seam a wrapper owns is `check` → `CheckedSender::send`.
pub(super) struct TwoRevealChecked<S> {
    inner: S,
    recipients: Recipients<PeerPubkey>,
    report: ByzReport,
    cfg: Option<TwoRevealCfg>,
}

/// The wrapper's own send error: the inner error type is generic and cannot be
/// named across the `Checked<'a>` lifetime, so it is flattened to its `Debug` text.
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
        if matches!(self.recipients, Recipients::All) {
            note_confirm(&self.report, bytes.as_ref());
        }
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

    /// The rate check is the inner sender's own: the stand's channels are registered
    /// at `Quota::per_second(NonZeroU32::MAX)`, so no recipient is ever filtered and
    /// admitting here changes nothing about what goes on the wire.
    async fn check(
        &mut self,
        recipients: Recipients<Self::PublicKey>,
    ) -> Result<Self::Checked<'_>, SystemTime> {
        Ok(TwoRevealChecked {
            inner: self.inner.clone(),
            recipients,
            report: self.report.clone(),
            cfg: self.cfg.clone(),
        })
    }
}

/// The plane's [`Beacon`] with one operation changed: the signer scheme is rebuilt
/// over the epoch's verify-only oracle, so this node produces no seed partial and
/// therefore casts no vote.
pub(super) struct WithholdingRandomness {
    inner: Arc<dyn Beacon>,
    chain_id: u64,
    report: ByzReport,
}

impl WithholdingRandomness {
    pub(super) fn wrap(
        inner: Arc<dyn Beacon>,
        chain_id: u64,
        report: ByzReport,
    ) -> Arc<dyn Beacon> {
        Arc::new(Self {
            inner,
            chain_id,
            report,
        })
    }
}

impl Beacon for WithholdingRandomness {
    fn seed(&self, round: Round) -> Option<Seed> {
        self.inner.seed(round)
    }

    fn terminal_seed(&self, round: Round) -> Option<Seed> {
        self.inner.terminal_seed(round)
    }

    fn mandatory_at(&self, epoch: u64) -> bool {
        self.inner.mandatory_at(epoch)
    }

    fn can_participate(&self, epoch: Epoch) -> ShareProbe {
        self.inner.can_participate(epoch)
    }

    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        self.inner.oracle_for(epoch)
    }

    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
        self.inner.ensure_key(epoch, effort)
    }

    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
        self.inner.observe_certificate(cert)
    }

    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>> {
        self.inner.artifact_bytes(epoch)
    }

    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch) {
        self.inner.observe_epoch(reconciled, entered_frontier)
    }

    fn observe_cert(&self, epoch: u64) {
        self.inner.observe_cert(epoch)
    }

    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
        self.inner.subscribe()
    }

    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        self.inner.faults()
    }

    /// The inner verdict is taken first — it carries the beacon's own side effects,
    /// including the key publication — and only a `Signs` arm is rebuilt.
    fn signer(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        let honest = match self.inner.signer(epoch, snap, keypair) {
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
        // The withholding's own witness: the honest scheme signs a probe subject of
        // this epoch and the rebuilt one does not. A `(false, false)` would mean the
        // node had no partial to withhold in the first place.
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

/// The heights whose served certificates get a forged σ slot: the first block of
/// the bootstrap epoch (`2 * epoch_len = 64`) and the six after it.
pub(super) const FORGE_WINDOW: std::ops::RangeInclusive<u64> = 64..=70;

/// The heights whose `Finalized{h}` by-height pulls a
/// [`ForgeMode::WrongHeightFinalized`] role answers with the `h − 1` pair. Chosen to
/// span the whole catch-up gap of a validator dropped from the committee at the first
/// boundary, starving every by-height repair in the range so the gap never closes.
pub(super) const WRONG_HEIGHT_WINDOW: std::ops::RangeInclusive<u64> = 6..=40;

/// How much a [`ForgeMode::InflatedLatest`] adds to the served `Latest` tip height:
/// far above any real chain height, so the re-jump backfills the real prefix and then
/// cannot resolve the inflated landing height, which is `SyncFailure::Stalled` rather
/// than the frozen-head `StalledWithPeers`.
pub(super) const LATEST_INFLATION: u64 = 1_000_000;

/// Where a [`ForgeMode::LyingLatest`] divergent branch forks: above the victim's
/// rotation-boundary park, so the victim's own canonical chain has no block there to
/// conflict with and the walk finds its fork point at the shared honest prefix.
pub(super) const LYING_DIVERGE_AT: u64 = 100;

/// Everything the lying-upstream role ([`ForgeMode::LyingLatest`]) needs to seed a
/// divergent branch into the shared devp2p peer network and point its served `Latest`
/// at it. Only built for that role (`None` otherwise).
#[derive(Clone)]
pub(super) struct LyingCfg {
    /// This node's own chain, read for the honest fork parent hash at
    /// `LYING_DIVERGE_AT - 1`.
    pub chain: FakeChain,
    /// The shared devp2p peer network the divergent branch is published into.
    pub el_network: ElNetwork,
}

/// The deterministic hash of the divergent branch's block at `height` — a hash the
/// honest chain never produces, so it coexists with the honest hash at the same height
/// in the hash-keyed [`ElNetwork`].
pub(super) fn divergent_hash(height: u64) -> B256 {
    keccak256([b"lying-upstream-div".as_slice(), &height.to_be_bytes()].concat())
}

/// What the frontier-plane `Producer` half of one node forges. `Role`-derived in
/// `stand::build_node`; a wrapper on an honest node is [`ForgeMode::Watch`] and
/// changes nothing on the wire.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ForgeMode {
    /// Not forging: watch what this node serves so a follower relaying a forgery it
    /// accepted is observable ([`ByzFacts::served_seed_replays`]).
    Watch,
    /// Replace the σ slot of the `Finalized{h}` certificates served inside
    /// [`FORGE_WINDOW`], multisig half untouched.
    SeedSlot,
    /// Inflate the `Latest` answer's `block.height` by [`LATEST_INFLATION`],
    /// re-pointing `proposal.payload` to the new digest so
    /// [`verify_jump_structural`] still passes; `result` unchanged.
    InflatedLatest,
    /// Seed a divergent branch into the shared [`ElNetwork`] (via [`LyingCfg`] /
    /// `publish_branch`) and replace the `Latest` answer's `block.result` with that
    /// branch's tip, re-pointing `proposal.payload` to the new digest so
    /// [`verify_jump_structural`] still passes. The multisig half is left as the real
    /// signature over the original payload, so the jump lands the now-servable branch
    /// and then fails `verify_jump_authenticated`.
    LyingLatest,
    /// Answer a `Finalized{h}` pull (for `h` in [`WRONG_HEIGHT_WINDOW`]) with this
    /// node's own valid pair of height `h − 1` instead of `h`. Nothing is mutated —
    /// the pair is a wholly real, self-consistent finalization of the wrong height.
    /// `FrontierHandler::deliver` fans it to the `Finalized{h}` waiter and returns
    /// `true`; the marshal then rejects `block.height() != h` and the gap at `h`
    /// never closes.
    WrongHeightFinalized,
}

#[derive(Default)]
struct ForgeState {
    /// A real σ of some other round of the same epoch, harvested from an earlier
    /// answer: a valid G1 point that cannot verify for the round it is planted into.
    harvested: Option<(Round, BlsSignature)>,
    /// Every `(σ, the round it was served under)` this node has served — the
    /// archive-poisoning witness of a node that is not forging.
    served: std::collections::HashMap<Vec<u8>, Round>,
}

/// The frontier plane's `Producer` + `Consumer` with a forging `produce`.
#[derive(Clone)]
pub(super) struct ForgedSeedProducer {
    inner: CountingHandler,
    state: Arc<Mutex<ForgeState>>,
    report: ByzReport,
    mode: ForgeMode,
    /// Present only for [`ForgeMode::LyingLatest`].
    lying: Option<LyingCfg>,
}

impl ForgedSeedProducer {
    pub(super) fn new(
        inner: CountingHandler,
        report: ByzReport,
        mode: ForgeMode,
        lying: Option<LyingCfg>,
    ) -> Self {
        Self {
            inner,
            state: Arc::new(Mutex::new(ForgeState::default())),
            report,
            mode,
            lying,
        }
    }

    /// Re-serve a `Latest` answer with `block` mutated by `mutate` and the
    /// certificate's `proposal.payload` re-pointed to the mutated block's digest, so
    /// [`verify_jump_structural`] still passes; the multisig half is left
    /// byte-identical. Runs the tamper's self-check on the wire bytes before allowing
    /// the swap. `record` writes the mode's [`ByzFacts`] fields.
    fn forge_latest(
        &self,
        bytes: &Bytes,
        mutate: impl Fn(&mut OrderBlock),
        record: impl Fn(&mut super::fakes::ByzFacts, &OrderBlock, &OrderBlock, bool),
    ) -> Option<Vec<u8>> {
        let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
        let (fin, block) = FrontierPair::decode_cfg(bytes.as_ref(), &(cap, ())).ok()?;
        let mut forged_block = block.clone();
        mutate(&mut forged_block);
        if forged_block == block {
            return None;
        }
        // Re-point the payload so the structural gate passes; keep round/parent and
        // the whole (real) multisig certificate.
        let forged = Finalization::<BlsScheme, Digest> {
            proposal: Proposal {
                round: fin.proposal.round,
                parent: fin.proposal.parent,
                payload: forged_block.digest(),
            },
            certificate: fin.certificate.clone(),
        };
        let wire = (forged, forged_block.clone()).encode().to_vec();

        // Self-check on the bytes that go out, decoded back the way the victim's
        // `decode_frontier` will, before the swap is allowed.
        let (back_fin, back_block) = FrontierPair::decode_cfg(wire.as_slice(), &(cap, ()))
            .expect("the forged frontier pair must decode");
        assert_ne!(
            back_block, block,
            "the Latest forge changed nothing on the served block"
        );
        let structural_ok = verify_jump_structural(&UpstreamFinalized {
            finalization: back_fin,
            block: back_block.clone(),
        })
        .is_ok();
        assert!(
            structural_ok,
            "the forged Latest answer does not pass verify_jump_structural"
        );
        self.report
            .with(|f| record(f, &block, &back_block, structural_ok));
        Some(wire)
    }

    /// Re-serve `bytes` with the σ slot replaced, or `None` when there is nothing to
    /// replace it with yet. Runs the tamper's own self-check on the result.
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
                // the next round's forgery.
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

        // Self-check on the bytes that go out, not on the struct we built: decode
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
    /// Inflate the served `Latest` tip height by [`LATEST_INFLATION`].
    fn forge_inflated_latest(&self, bytes: &Bytes) -> Option<Vec<u8>> {
        self.forge_latest(
            bytes,
            |b| b.height += LATEST_INFLATION,
            |f, orig, forged, structural_ok| {
                f.latest_inflated += 1;
                f.inflate_from = Some(orig.height);
                f.inflate_to = Some(forged.height);
                f.inflate_delta_ok = f.latest_inflated == 1 || f.inflate_delta_ok;
                f.inflate_structural_ok = f.latest_inflated == 1 || f.inflate_structural_ok;
                f.inflate_delta_ok &= forged.height == orig.height + LATEST_INFLATION;
                f.inflate_structural_ok &= structural_ok;
            },
        )
    }

    /// Seed a divergent branch into the shared [`ElNetwork`] (forking at
    /// [`LYING_DIVERGE_AT`], above the victim's park, and extending to the served
    /// tip's landing height `H = block.height - K`) and replace the served `Latest`
    /// tip's `result` with that branch's tip hash. The branch is now servable, so the
    /// victim's re-jump can EL-sync onto it and reach `verify_jump_authenticated` —
    /// which fails, because the multisig is the real one over the original payload.
    fn forge_lying_latest(&self, bytes: &Bytes) -> Option<Vec<u8>> {
        let lying = self.lying.as_ref()?;
        // The landing height the served tip claims: `block.height - K`.
        let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
        let (_, block) = FrontierPair::decode_cfg(bytes.as_ref(), &(cap, ())).ok()?;
        let landing = block.height.checked_sub(crate::order_block::K)?;
        if landing < LYING_DIVERGE_AT {
            // Too early: the fork point is not yet below the landing — nothing to
            // diverge onto. Serve honest.
            return None;
        }
        // The fork parent: this node's own honest hash at `LYING_DIVERGE_AT - 1`,
        // already in `ElNetwork`, so the victim can walk the divergent branch down to
        // it and then along the honest prefix to its own fork point.
        let fork_parent = lying.chain.hash_at(LYING_DIVERGE_AT - 1)?;
        // Publish (idempotently) the divergent branch `LYING_DIVERGE_AT ..= landing`.
        let branch: Vec<(B256, u64, B256)> = (LYING_DIVERGE_AT..=landing)
            .map(|h| {
                let parent = if h == LYING_DIVERGE_AT {
                    fork_parent
                } else {
                    divergent_hash(h - 1)
                };
                (divergent_hash(h), h, parent)
            })
            .collect();
        lying.el_network.publish_branch(&branch);
        self.forge_latest(
            bytes,
            |b| b.result = divergent_hash(landing),
            |f, orig, forged, structural_ok| {
                f.result_forged += 1;
                f.forged_result_from = Some(orig.result);
                f.forged_result_to = Some(forged.result);
                f.result_differs = f.result_forged == 1 || f.result_differs;
                f.result_structural_ok = f.result_forged == 1 || f.result_structural_ok;
                f.result_differs &= forged.result != orig.result;
                f.result_structural_ok &= structural_ok;
            },
        )
    }

    /// The archive-poisoning witness. A σ is unique per `(round, PK)`, so serving the
    /// same σ under two different rounds means this node stored a certificate whose σ
    /// slot had been swapped and is now relaying it.
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

    /// Record one wrong-height substitution: the served pair is a wholly real,
    /// self-consistent finalization whose height is exactly `requested - 1`. Both are
    /// asserted before the answer is allowed out.
    fn record_wrong_height(&self, requested: u64, bytes: &Bytes) {
        let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
        let (fin, block) = FrontierPair::decode_cfg(bytes.as_ref(), &(cap, ()))
            .expect("the wrong-height pair must decode");
        let served = block.height;
        assert_eq!(
            served,
            requested - 1,
            "the wrong-height role served height {served}, not requested-1 ({})",
            requested - 1
        );
        let self_consistent = fin.proposal.payload == block.digest();
        assert!(
            self_consistent,
            "the served wrong-height pair is not a real finalization (payload != digest)"
        );
        self.report.with(|f| {
            f.wrong_height_served += 1;
            f.wrong_height_pairs.push((requested, served));
            f.wrong_height_valid = (f.wrong_height_served == 1 || f.wrong_height_valid)
                && served == requested - 1
                && self_consistent;
        });
    }
}

impl Producer for ForgedSeedProducer {
    type Key = FrontierKey;

    async fn produce(&mut self, key: FrontierKey) -> cw_oneshot::Receiver<Bytes> {
        let rx = self.inner.produce(key).await;
        // `FrontierHandler::produce` fills the channel inline before returning it, so
        // awaiting `rx` never parks; an archive miss shows up as a dropped sender.
        match (key, self.mode) {
            // Forge the σ slot of the in-window Finalized certs.
            (FrontierKey::Finalized { height }, ForgeMode::SeedSlot) => {
                if !FORGE_WINDOW.contains(&height) {
                    return rx;
                }
                let Ok(bytes) = rx.await else {
                    return dropped();
                };
                resolved(self.forge(height, &bytes).map(Bytes::from).unwrap_or(bytes))
            }
            // Answer Finalized{h} with this node's own valid pair of h-1.
            (FrontierKey::Finalized { height }, ForgeMode::WrongHeightFinalized) => {
                if !WRONG_HEIGHT_WINDOW.contains(&height) || height == 0 {
                    return rx; // out of window / h0: serve the honest h pair
                }
                // Discard the h answer we opened and serve this node's own real pair
                // of h-1 instead — only the wrong height is sent. The extra produce is
                // benign under the deterministic runner.
                drop(rx);
                let lower = FrontierKey::Finalized { height: height - 1 };
                let Ok(bytes) = self.inner.produce(lower).await.await else {
                    return dropped();
                };
                self.record_wrong_height(height, &bytes);
                resolved(bytes)
            }
            // Every other mode watches the Finalized answers it serves (the
            // archive-poisoning witness a non-forging follower keeps).
            (FrontierKey::Finalized { height }, _) => {
                let Ok(bytes) = rx.await else {
                    return dropped();
                };
                self.note_served(height, &bytes);
                resolved(bytes)
            }
            // Inflate the served Latest tip height.
            (FrontierKey::Latest, ForgeMode::InflatedLatest) => {
                let Ok(bytes) = rx.await else {
                    return dropped();
                };
                resolved(
                    self.forge_inflated_latest(&bytes)
                        .map(Bytes::from)
                        .unwrap_or(bytes),
                )
            }
            // Replace the served Latest tip's result.
            (FrontierKey::Latest, ForgeMode::LyingLatest) => {
                let Ok(bytes) = rx.await else {
                    return dropped();
                };
                resolved(
                    self.forge_lying_latest(&bytes)
                        .map(Bytes::from)
                        .unwrap_or(bytes),
                )
            }
            (FrontierKey::Latest, _) => rx,
        }
    }
}

/// A oneshot receiver pre-filled with `bytes` (the serve side answers inline).
fn resolved(bytes: Bytes) -> cw_oneshot::Receiver<Bytes> {
    let (tx, rx) = cw_oneshot::channel::<Bytes>();
    let _ = tx.send(bytes);
    rx
}

/// A oneshot receiver whose sender was dropped — served on as "no data".
fn dropped() -> cw_oneshot::Receiver<Bytes> {
    let (tx, rx) = cw_oneshot::channel::<Bytes>();
    drop(tx);
    rx
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
