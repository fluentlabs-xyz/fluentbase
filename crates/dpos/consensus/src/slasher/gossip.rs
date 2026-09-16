//! The evidence channel: the votes a node republishes so the two halves of a
//! split-delivered equivocation can meet.
//!
//! An equivocator that sends each half of its double-signature to a disjoint set of
//! peers is invisible to simplex — no single node ever holds both, so no
//! `Conflicting*` activity is raised and the certificate path yields no evidence.
//! This channel is the meeting place: when a round is decided against the votes a
//! node is holding for it, that node broadcasts them on `EVIDENCE_CHANNEL`, and
//! every receiver verifies each signature and feeds it into its own slasher vote
//! store, where the pair assembles into a charge.
//!
//! The p2p halves live in the node crate, so nothing here touches them:
//! [`EvidenceBridge`] is the seam.

use crate::{
    digest::Digest,
    slasher::{
        actor::EpochCursor,
        evidence::verify_pre_submit_vote_only,
        ingress::{GossipSink, Mailbox, Message},
    },
};
use bytes::Bytes;
use commonware_codec::{Decode as _, Encode as _, Error as CodecError, RangeCfg};
use commonware_consensus::{
    simplex::types::{Activity, Vote},
    Epochable, Viewable,
};
use fluentbase_bls::{
    fluent_namespace, EpochCommittee, PeerPubkey, Scheme as BlsScheme, VoteScheme,
};
use fluentbase_p2p::{constants::MAX_COMMITTEE_SIZE, Ingress, TrackedWindow};
use rand_core::OsRng;
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// The `channel` label every evidence ingress refusal is counted under.
pub(crate) const EVIDENCE_CHANNEL_LABEL: &str = "evidence";

/// Wire payload: the votes one node republishes for a single round.
///
/// A batch rather than a single vote, because `EVIDENCE_QUOTA` budgets one
/// publication per validator per failed view while a failed view can leave a
/// node holding one vote per committee member.
pub type EvidenceBatch = Vec<Vote<BlsScheme, Digest>>;

/// The epoch's BLS committee, resolved by the node from its own staking state.
/// A forwarded vote is checked against this and nothing else.
pub type EvidenceCommitteeFor = Arc<dyn Fn(u64) -> Option<EpochCommittee> + Send + Sync>;

/// Decode bound: one round yields at most one vote per committee member per
/// kind, so a longer batch is a peer buying unbounded work from us.
fn batch_cfg() -> (RangeCfg<usize>, ()) {
    ((0..=2 * MAX_COMMITTEE_SIZE as usize).into(), ())
}

pub fn encode_batch(votes: &EvidenceBatch) -> Bytes {
    votes.encode()
}

pub fn decode_batch(bytes: &[u8]) -> Result<EvidenceBatch, CodecError> {
    EvidenceBatch::decode_cfg(bytes, &batch_cfg())
}

/// A vote and the `Activity` the slasher's vote store keys on are the same
/// object under two names.
fn into_activity(vote: Vote<BlsScheme, Digest>) -> Message {
    match vote {
        Vote::Notarize(n) => Activity::Notarize(n),
        Vote::Nullify(n) => Activity::Nullify(n),
        Vote::Finalize(f) => Activity::Finalize(f),
    }
}

/// Check one vote's own signature against the epoch's multisig verifier.
///
/// `Activity::verified()` is false for every vote variant (commonware
/// `simplex/types.rs`), so nothing upstream of here has checked it — neither for
/// a vote the local engine reported nor for one a peer forwarded.
pub(crate) fn verify_vote(vote: &Vote<BlsScheme, Digest>, scheme: &VoteScheme) -> bool {
    verify_pre_submit_vote_only(&into_activity(vote.clone()), scheme, &mut OsRng).is_ok()
}

/// Decode, bound, verify and report one peer's evidence batch.
///
/// Verification here is what keeps an unverified vote out of the store, where it
/// would be republished on the next trigger and could pair into a charge against a
/// validator that signed nothing.
///
/// The sender bound comes first. Evidence is committee traffic: a peer that sits in
/// none of the three tracked committee records (`C[E−1] ∪ C[E] ∪ C[E+1]`) has no
/// round of its own to republish, so it is refused before the decode.
///
/// The epoch bound is the other half. Nothing about a forwarded vote constrains the
/// epoch it names — its signer picks that — so the batch is checked against the
/// window the vote store actually retains ([`EpochCursor::retains`]) and against the
/// sender's own membership mask before `committee_for` is called. Resolving first
/// would mean one `getEpochCommitteeWithStakes` state read per message for any epoch
/// a peer cares to name, ahead of any signature check.
pub fn ingest_batch(
    from: &PeerPubkey,
    bytes: &[u8],
    chain_id: u64,
    committee_for: &EvidenceCommitteeFor,
    bridge: &EvidenceBridge,
    window: &TrackedWindow,
) {
    // Before the decode: a sender outside the tracked set buys nothing at all here.
    // `None` = no peer set registered yet, which is not a verdict.
    let ingress = window.classify(from);
    if matches!(ingress, Some(Ingress::Dropped) | Some(Ingress::Tracked(_))) {
        let reason = ingress.as_ref().map_or("none", Ingress::refusal);
        debug!(%from, reason, "evidence: sender is not a committee member; dropping");
        crate::dpos::record_ingress_drop(EVIDENCE_CHANNEL_LABEL, reason);
        return;
    }
    let batch = match decode_batch(bytes) {
        Ok(batch) => batch,
        Err(e) => {
            debug!(?e, "evidence: undecodable batch; dropping");
            metrics::counter!("slasher_evidence_undecodable_total").increment(1);
            return;
        }
    };
    let Some(first) = batch.first() else {
        return;
    };
    let (epoch, view) = (first.epoch().get(), first.view().get());
    // One batch, one round: a mixed batch would cost one committee resolve per vote,
    // which is exactly the unbounded work the decode bound refuses.
    if batch
        .iter()
        .any(|v| v.epoch().get() != epoch || v.view().get() != view)
    {
        debug!(
            epoch,
            view, "evidence: batch spans several rounds; dropping"
        );
        metrics::counter!("slasher_evidence_undecodable_total").increment(1);
        return;
    }
    if !bridge.epoch_cursor().retains(epoch) {
        debug!(
            epoch,
            current = bridge.epoch_cursor().get(),
            "evidence: batch epoch outside the retained window; dropping"
        );
        metrics::counter!("slasher_evidence_out_of_window_total").increment(1);
        return;
    }
    // The sender must be in the record of the epoch it is republishing for, not
    // merely in one of the three. Still before `committee_for`.
    if let Some(ingress) = ingress.as_ref() {
        if !ingress.member_of(epoch) {
            debug!(
                %from,
                epoch, "evidence: sender is not in that epoch's committee; dropping"
            );
            crate::dpos::record_ingress_drop(EVIDENCE_CHANNEL_LABEL, "epoch");
            return;
        }
    }
    // Before the consensus layer launches there is no vote store to fill.
    let Some(sink) = bridge.gossip_sink() else {
        return;
    };
    let Some(committee) = committee_for(epoch) else {
        debug!(epoch, "evidence: committee unresolved; dropping batch");
        return;
    };
    let scheme = VoteScheme::verifier(&fluent_namespace(chain_id), committee.bimap.clone());
    for vote in batch {
        if !verify_vote(&vote, &scheme) {
            warn!(epoch, view, "evidence: forwarded vote failed verify");
            metrics::counter!("slasher_evidence_rejected_total").increment(1);
            continue;
        }
        metrics::counter!("slasher_evidence_accepted_total").increment(1);
        sink.report_gossiped(into_activity(vote));
    }
}

/// The two directions of the evidence channel, as the slasher sees them, plus the
/// one piece of slasher state the inbound direction has to read.
///
/// Outbound, the slasher pushes encoded batches into `publisher` and the node's
/// evidence task — which owns both p2p halves — broadcasts them. Inbound, `sink` is
/// the return path: [`crate::slasher::Actor::init`] fills it with the gossip half of
/// its own mailbox, because that mailbox does not exist until the consensus layer
/// launches.
///
/// `cursor` travels the same seam in the opposite direction: the actor writes it from
/// engine activity and [`ingest_batch`] reads it to bound what a peer may claim. It
/// is created here rather than in the actor for the same reason as `sink`.
#[derive(Clone)]
pub struct EvidenceBridge {
    publisher: mpsc::UnboundedSender<Bytes>,
    sink: Arc<OnceLock<GossipSink>>,
    cursor: EpochCursor,
}

impl EvidenceBridge {
    /// Build the bridge and the receiving end the node's evidence task drains.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Bytes>) {
        let (publisher, rx) = mpsc::unbounded_channel();
        (
            Self {
                publisher,
                sink: Arc::new(OnceLock::new()),
                cursor: EpochCursor::default(),
            },
            rx,
        )
    }

    /// The gossip half of the slasher's mailbox, once the consensus layer has
    /// launched. `None` before that, and a batch arriving in that window is
    /// simply dropped — the node has no vote store to put it in yet.
    ///
    /// A [`GossipSink`] and not a `Mailbox`: whatever goes in here is stamped
    /// [`crate::slasher::ingress::Provenance::Gossip`] and cannot claim to be an
    /// engine report.
    pub fn gossip_sink(&self) -> Option<&GossipSink> {
        self.sink.get()
    }

    /// The slasher's epoch cursor — read-only to everyone on this side of the
    /// bridge. See [`EpochCursor`].
    pub fn epoch_cursor(&self) -> &EpochCursor {
        &self.cursor
    }

    pub(super) fn bind_slasher(&self, mailbox: &Mailbox) {
        let _ = self.sink.set(mailbox.gossip_sink());
    }

    pub(super) fn publish(&self, batch: Bytes) {
        if self.publisher.send(batch).is_err() {
            warn!("evidence: publisher closed; equivocation gossip is down");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Notarize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random;
    use commonware_utils::{ordered::BiMap, TryCollect};
    use fluentbase_bls::{keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey, PeerPubkey};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    const TEST_CHAIN_ID: u64 = 20_994;
    const TEST_EPOCH: u64 = 7;

    fn signer_and_committee_at(seed: u64, n: usize, epoch: u64) -> (BlsScheme, EpochCommittee) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, BlsPubkey> = peer_sks
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
        let committee = EpochCommittee::from_unverified(TEST_EPOCH, bimap);
        let signer = build_signer(
            &fluent_namespace(TEST_CHAIN_ID),
            committee.bimap.clone(),
            &bls_kps[0],
            epoch,
            None,
        )
        .expect("signer is a committee member");
        (signer, committee)
    }

    /// A signer bound to `TEST_EPOCH`. A scheme refuses a subject from any other
    /// epoch, so a batch aimed at a different one needs its own signer.
    fn signer_and_committee(seed: u64, n: usize) -> (BlsScheme, EpochCommittee) {
        signer_and_committee_at(seed, n, TEST_EPOCH)
    }

    fn notarize_at(signer: &BlsScheme, epoch: u64, tag: u8) -> Vote<BlsScheme, Digest> {
        let round = Round::new(Epoch::new(epoch), View::new(42));
        Vote::Notarize(
            Notarize::sign(
                signer,
                Proposal::new(
                    round,
                    View::new(41),
                    Digest(alloy_primitives::B256::repeat_byte(tag)),
                ),
            )
            .expect("signs"),
        )
    }

    fn notarize(signer: &BlsScheme, tag: u8) -> Vote<BlsScheme, Digest> {
        notarize_at(signer, TEST_EPOCH, tag)
    }

    #[test]
    fn a_batch_round_trips_and_its_votes_verify() {
        let (signer, committee) = signer_and_committee(1, 4);
        let batch = vec![notarize(&signer, 0xaa), notarize(&signer, 0xbb)];
        let encoded = encode_batch(&batch);
        let decoded = decode_batch(&encoded).expect("round trip");
        assert_eq!(encode_batch(&decoded), encoded);

        let scheme = VoteScheme::verifier(&fluent_namespace(TEST_CHAIN_ID), committee.bimap);
        assert!(decoded.iter().all(|v| verify_vote(v, &scheme)));
    }

    #[test]
    fn a_vote_signed_under_another_chains_namespace_is_rejected() {
        let (signer, committee) = signer_and_committee(2, 4);
        let scheme = VoteScheme::verifier(&fluent_namespace(TEST_CHAIN_ID + 1), committee.bimap);
        assert!(!verify_vote(&notarize(&signer, 0xaa), &scheme));
    }

    #[test]
    fn a_batch_longer_than_the_committee_bound_is_refused() {
        let (signer, _) = signer_and_committee(3, 4);
        let over = vec![notarize(&signer, 0xaa); 2 * MAX_COMMITTEE_SIZE as usize + 1];
        assert!(decode_batch(&encode_batch(&over)).is_err());
    }

    /// The evidence sender bound: a peer in no tracked committee record buys
    /// nothing, and a peer in one record does not get to speak for another.
    ///
    /// Without it the epoch window is the only bound, so any tracked peer could name
    /// any retained epoch and buy one `getEpochCommitteeWithStakes` read per message
    /// ahead of any signature check.
    #[test]
    fn an_evidence_batch_from_outside_the_epochs_committee_resolves_nothing() {
        let (signer, committee) = signer_and_committee(4, 4);
        let (bridge, _publications) = EvidenceBridge::new();
        let (tx, mut delivered) = mpsc::unbounded_channel();
        bridge.bind_slasher(&crate::slasher::ingress::test_only_mailbox(tx));
        bridge.epoch_cursor().advance(TEST_EPOCH);

        let resolves = Arc::new(AtomicUsize::new(0));
        // The window this node registered: the members are primary for TEST_EPOCH,
        // one more peer is primary for TEST_EPOCH − 1 only, and an outsider is in
        // the registry and in no committee.
        let members: Vec<PeerPubkey> = committee.bimap.iter().cloned().collect();
        let committee_for: EvidenceCommitteeFor = {
            let resolves = resolves.clone();
            Arc::new(move |_| {
                resolves.fetch_add(1, AtomicOrdering::Relaxed);
                Some(committee.clone())
            })
        };
        let previous_only = Ed25519PrivateKey::from_seed(0xa1).public_key();
        let outsider = Ed25519PrivateKey::from_seed(0xa2).public_key();
        let window = TrackedWindow::default();
        window.record(
            TEST_EPOCH,
            &fluentbase_staking_reader::TrackedPeers {
                committees: vec![
                    (
                        TEST_EPOCH - 1,
                        commonware_utils::ordered::Set::from_iter_dedup([previous_only.clone()]),
                    ),
                    (
                        TEST_EPOCH,
                        commonware_utils::ordered::Set::from_iter_dedup(members.iter().cloned()),
                    ),
                ],
                secondary: commonware_utils::ordered::Set::from_iter_dedup([outsider.clone()]),
            },
        );

        let batch = encode_batch(&vec![notarize_at(&signer, TEST_EPOCH, 0xaa)]);

        // A registry-tier sender: refused before the decode.
        ingest_batch(
            &outsider,
            &batch,
            TEST_CHAIN_ID,
            &committee_for,
            &bridge,
            &window,
        );
        assert_eq!(
            resolves.load(AtomicOrdering::Relaxed),
            0,
            "a registry-tier sender must not buy a committee state read"
        );
        assert!(
            delivered.try_recv().is_err(),
            "and must not reach the store"
        );

        // A member of the outgoing committee speaking for the current one: in the
        // window, but not in this epoch's record.
        ingest_batch(
            &previous_only,
            &batch,
            TEST_CHAIN_ID,
            &committee_for,
            &bridge,
            &window,
        );
        assert_eq!(
            resolves.load(AtomicOrdering::Relaxed),
            0,
            "membership in another epoch's record is not membership in this one"
        );
        assert!(
            delivered.try_recv().is_err(),
            "and must not reach the store"
        );

        // A member of the epoch it republishes for: admitted, exactly as before.
        ingest_batch(
            &members[0],
            &batch,
            TEST_CHAIN_ID,
            &committee_for,
            &bridge,
            &window,
        );
        assert_eq!(resolves.load(AtomicOrdering::Relaxed), 1);
        assert!(
            delivered.try_recv().is_ok(),
            "a member's forwarded vote still lands"
        );
    }

    /// The epoch a forwarded batch names is chosen by its sender, and under the
    /// two-epoch ahead-commit horizon a committee for `E+2` is already on chain —
    /// so `committee_for` would happily resolve one, before a single signature had
    /// been checked. The bound has to sit in front of that read, not behind it.
    #[test]
    fn a_batch_outside_the_retained_window_is_refused_before_any_committee_is_resolved() {
        let (signer, committee) = signer_and_committee(4, 4);
        let (bridge, _publications) = EvidenceBridge::new();
        let (tx, mut delivered) = mpsc::unbounded_channel();
        bridge.bind_slasher(&crate::slasher::ingress::test_only_mailbox(tx));
        // The local engine has consensus at TEST_EPOCH, so the vote store retains
        // TEST_EPOCH-1 ..= TEST_EPOCH and nothing else.
        bridge.epoch_cursor().advance(TEST_EPOCH);

        let resolves = Arc::new(AtomicUsize::new(0));
        let committee_for: EvidenceCommitteeFor = {
            let resolves = resolves.clone();
            Arc::new(move |_| {
                resolves.fetch_add(1, AtomicOrdering::Relaxed);
                Some(committee.clone())
            })
        };

        let ahead = {
            let (ahead_signer, _) = signer_and_committee_at(1, 4, TEST_EPOCH + 2);
            encode_batch(&vec![notarize_at(&ahead_signer, TEST_EPOCH + 2, 0xaa)])
        };
        let anyone = Ed25519PrivateKey::from_seed(0x5e).public_key();
        let open = TrackedWindow::default();
        ingest_batch(
            &anyone,
            &ahead,
            TEST_CHAIN_ID,
            &committee_for,
            &bridge,
            &open,
        );
        assert_eq!(
            resolves.load(AtomicOrdering::Relaxed),
            0,
            "an out-of-window epoch must not buy a committee state read"
        );
        assert!(
            delivered.try_recv().is_err(),
            "and must not reach the vote store"
        );

        // The same message inside the window is still resolved, verified and kept
        // — the bound rejects a claim, not the feature.
        let live = encode_batch(&vec![notarize_at(&signer, TEST_EPOCH, 0xaa)]);
        ingest_batch(
            &anyone,
            &live,
            TEST_CHAIN_ID,
            &committee_for,
            &bridge,
            &open,
        );
        assert_eq!(resolves.load(AtomicOrdering::Relaxed), 1);
        let entry = delivered.try_recv().expect("an in-window vote lands");
        assert_eq!(
            entry.provenance,
            crate::slasher::ingress::Provenance::Gossip
        );
    }
}
