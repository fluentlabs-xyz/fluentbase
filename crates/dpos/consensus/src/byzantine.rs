//! DEVNET/TEST-ONLY byzantine validator code, gated behind the
//! `dpos-devnet-byzantine` cargo feature (also built under `test`, so the in-crate
//! forge/certify unit tests can reach [`forge_outcome_same_committee`]). Never
//! compiled into a production build. This is the SINGLE home for byzantine logic:
//! the behaviour selector [`ByzantineMode`], the beacon-`PK_E` forge
//! [`forge_outcome_same_committee`], and the vote double-signer [`VoteEquivocator`].
//!
//! [`VoteEquivocator`] is a vote-reactive double-signer adapted from commonware's
//! `simplex::mocks::conflicter::Conflicter`. It is vendored (rather than reused via
//! the upstream `mocks` feature) for two reasons: (1) the upstream `mocks` feature
//! transitively pulls `commonware-{cryptography,p2p,resolver}/mocks`; (2) the
//! upstream `Conflicter<E, S, H>` ties the vote digest to a `Hasher`'s `H::Digest`,
//! whereas our consensus digest [`crate::digest::Digest`] is a standalone `B256`
//! wrapper (it impls `commonware_cryptography::Digest` + `commonware_math::algebra::
//! Random` directly), so a vendored actor over `(BlsScheme, Digest)` needs no
//! `Hasher` shim.
//!
//! Behaviour: for every `Notarize` / `Finalize` vote it receives on the vote
//! channel, it signs and broadcasts TWO conflicting votes for the SAME round (one
//! over a random proposal, one over the received proposal). Honest peers observe
//! the two same-round / differing-proposal votes from the same signer, report a
//! `ConflictingNotarize` / `ConflictingFinalize` activity to their slasher, and the
//! offender is jailed on-chain. The trigger is VOTE-based (fires every view the
//! node participates in), NOT leadership-based — deterministic in docker.
//!
//! The actor REPLACES the honest `simplex::Engine` for the flagged node: it runs no
//! marshal/executor/slasher/EL. The flagged node never finalizes; the honest
//! quorum (n − f) does. Honest peers block the equivocator, but its first
//! conflicting pair is enough for the slash.

use crate::beacon::outcome::DkgOutcome;
use crate::digest::Digest;
use commonware_codec::{DecodeExt, Encode};
use commonware_consensus::{
    simplex::types::{Finalize, Notarize, Proposal, Vote},
    types::{Epoch, Round, View},
};
use commonware_math::algebra::Random;
use commonware_p2p::{Receiver, Recipients, Sender};
use commonware_runtime::{spawn_cell, Clock, ContextCell, Handle, Spawner};
use fluentbase_bls::Scheme as BlsScheme;
use rand_core::CryptoRngCore;
use tracing::{debug, warn};

/// DEVNET/TEST-ONLY byzantine validator behaviour selector. `None` (and absent
/// without the feature) on every honest node, so a deployed validator can never
/// misbehave through this path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByzantineMode {
    /// At a CHANGE-epoch first block: propose a FORGED `PK_E` (different from the
    /// real DKG outcome) AND, as a verifier, vote yes on any change-epoch boundary
    /// block regardless of the "C" gate (so a colluding byzantine quorum can
    /// notarize a forge and exercise the certify-hook Nullify path). See
    /// [`crate::beacon::certify`] and the byzantine-vrf smoke.
    ForgeBeaconPk,
    /// Multisig double-sign: this node EQUIVOCATES on every Notarize/Finalize it
    /// would cast (two conflicting votes — a random proposal + the real one — for
    /// the SAME round) so honest peers report a `ConflictingNotarize` /
    /// `ConflictingFinalize` and the slasher jails it. The trigger is VOTE-based
    /// (fires every view the node participates in), NOT leadership-based, so it is
    /// deterministic in docker. Consumed in [`crate::engine`] (the per-epoch engine
    /// swaps in a [`VoteEquivocator`] in place of the honest `simplex::Engine`).
    /// See the byzantine equivocation smoke.
    Equivocate,
}

/// DEVNET/TEST-ONLY forge of a different per-epoch DKG outcome over the SAME
/// committee. Deals a fresh anonymous DKG to `real.players()` (the proposer holds
/// the committee's PUBLIC peer set, never the other validators' shares) with a
/// fixed devnet RNG, yielding an `Output` whose `players()`/`total()` match the
/// real committee — so it passes the epoch-type/shape gate — but whose `PK_E`
/// differs. The forged polynomial does NOT thread the honest shares, so each honest
/// share-holder's "C" gate rejects it at verify; under the realistic `f=1` bound the
/// forge cannot reach a notarization quorum, so it never finalizes (the
/// consensus-level SAFETY observable). The certify hook ([`crate::beacon::certify`])
/// is the closure that would Nullify it IF a colluding byzantine quorum did notarize
/// it — exercised by the gated certify tests where the collusion is constructible.
pub fn forge_outcome_same_committee(real: &DkgOutcome) -> DkgOutcome {
    use commonware_cryptography::bls12381::{
        dkg::deal,
        primitives::{sharing::Mode, variant::MinSig},
    };
    use commonware_utils::N3f1;
    use fluentbase_bls::PeerPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    // Fixed devnet seed → deterministic forge (every byzantine node forges the
    // identical PK_E for a given committee, so they collude on one value).
    let mut rng = StdRng::seed_from_u64(0xB17E_FACE);
    let players = real.players().clone();
    let (forged, _shares) = deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
        .expect("forge: deal over the real committee's public players");
    debug_assert_ne!(
        forged.public().public(),
        real.public().public(),
        "a forged outcome must carry a different PK_E than the real one"
    );
    forged
}

/// Build the equivocation partner proposal: the SAME round and parent as the vote
/// we are conflicting with, but a fresh RANDOM payload — so the two votes differ in
/// proposal only (the definition of an equivocation an honest peer reports).
fn forged_proposal(round: Round, parent: View, rng: &mut impl CryptoRngCore) -> Proposal<Digest> {
    Proposal::new(round, parent, Digest::random(rng))
}

/// A byzantine validator that equivocates on every Notarize/Finalize it sees.
pub struct VoteEquivocator<E: Clock + CryptoRngCore + Spawner> {
    context: ContextCell<E>,
    scheme: BlsScheme,
}

impl<E: Clock + CryptoRngCore + Spawner> VoteEquivocator<E> {
    /// Build the equivocator from the per-epoch context and the node's own signer
    /// scheme (the same scheme the honest engine would have used to sign).
    pub fn new(context: E, scheme: BlsScheme) -> Self {
        Self {
            context: ContextCell::new(context),
            scheme,
        }
    }

    /// Spawn the equivocator on the per-epoch vote subchannel. Returns the task
    /// handle (joined by the per-epoch engine slot in [`crate::epoch_manager`]).
    pub fn start(
        mut self,
        vote: (
            impl Sender<PublicKey = commonware_cryptography::ed25519::PublicKey>,
            impl Receiver<PublicKey = commonware_cryptography::ed25519::PublicKey>,
        ),
    ) -> Handle<()> {
        warn!("BYZANTINE: equivocating votes (double-sign) — NEVER use in production");
        spawn_cell!(self.context, self.run(vote).await)
    }

    async fn run(
        mut self,
        vote: (
            impl Sender<PublicKey = commonware_cryptography::ed25519::PublicKey>,
            impl Receiver<PublicKey = commonware_cryptography::ed25519::PublicKey>,
        ),
    ) {
        // Probe once at startup: the engine gate that routes here
        // (`engine.rs`: `can_sign = member_signer.is_some() && can_sign_locally`)
        // admits a member that holds the local polynomial WITHOUT a share — its
        // `can_sign_locally` is `(Some(_), _) => true` regardless of the share — yet
        // the combined `sign()` self-suppresses every seed-bearing vote (`None`) for
        // such a node (combined_scheme.rs). It would then equivocate NOTHING while
        // also running no honest engine: a silent no-op. Warn LOUDLY instead, so the
        // smoke's jail assertion fails fast rather than hanging on a node that
        // quietly does nothing. The genesis smoke stack seeds the byzantine node, so
        // this is a misconfiguration guard, not the steady-state path.
        let probe = forged_proposal(
            Round::new(Epoch::new(0), View::new(0)),
            View::new(0),
            &mut self.context,
        );
        if Notarize::<BlsScheme, _>::sign(&self.scheme, probe).is_none() {
            warn!(
                "BYZANTINE: signer scheme cannot sign (beacon-active epoch with no local \
                 share?) — this node will NOT equivocate this epoch; check committee/share config"
            );
        }

        let (mut sender, mut receiver) = vote;
        while let Ok((from, msg)) = receiver.recv().await {
            let vote = match Vote::<BlsScheme, Digest>::decode(msg) {
                Ok(vote) => vote,
                Err(err) => {
                    debug!(?err, sender = ?from, "byzantine: failed to decode vote");
                    continue;
                }
            };
            match vote {
                Vote::Notarize(notarize) => {
                    // Conflicting Notarize over a RANDOM proposal (same round/parent),
                    // then the RECEIVED proposal — the conflicting partner.
                    let forged = forged_proposal(
                        notarize.round(),
                        notarize.proposal.parent,
                        &mut self.context,
                    );
                    self.broadcast_notarize(&mut sender, forged).await;
                    self.broadcast_notarize(&mut sender, notarize.proposal).await;
                }
                Vote::Finalize(finalize) => {
                    let forged = forged_proposal(
                        finalize.round(),
                        finalize.proposal.parent,
                        &mut self.context,
                    );
                    self.broadcast_finalize(&mut sender, forged).await;
                    self.broadcast_finalize(&mut sender, finalize.proposal).await;
                }
                Vote::Nullify(_) => continue,
            }
        }
    }

    /// Sign + broadcast a single Notarize. `sign` returns `None` when this node
    /// cannot sign (e.g. a beacon-active epoch where it holds no share — the
    /// combined scheme self-suppresses the seed-bearing vote); skip rather than
    /// panic so the mode is robust outside the seeded genesis stack. The startup
    /// probe in `run()` has already warned loudly for this case.
    async fn broadcast_notarize(
        &self,
        sender: &mut impl Sender<PublicKey = commonware_cryptography::ed25519::PublicKey>,
        proposal: Proposal<Digest>,
    ) {
        let Some(n) = Notarize::<BlsScheme, _>::sign(&self.scheme, proposal) else {
            debug!("byzantine: scheme could not sign Notarize (no local share); skipping");
            return;
        };
        let msg = Vote::Notarize(n).encode();
        if let Err(err) = sender.send(Recipients::All, msg, true).await {
            debug!(?err, "byzantine: failed to broadcast conflicting Notarize");
        }
    }

    async fn broadcast_finalize(
        &self,
        sender: &mut impl Sender<PublicKey = commonware_cryptography::ed25519::PublicKey>,
        proposal: Proposal<Digest>,
    ) {
        let Some(f) = Finalize::<BlsScheme, _>::sign(&self.scheme, proposal) else {
            debug!("byzantine: scheme could not sign Finalize (no local share); skipping");
            return;
        };
        let msg = Vote::Finalize(f).encode();
        if let Err(err) = sender.send(Recipients::All, msg, true).await {
            debug!(?err, "byzantine: failed to broadcast conflicting Finalize");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_utils::{ordered::BiMap, TryCollect};
    use fluentbase_bls::{
        fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
        scheme::build_verifier, BlsPubkey,
    };
    use rand_08::{rngs::StdRng, SeedableRng};

    // The genuinely new behaviour this actor adds: given a vote it received, it
    // produces a SECOND, conflicting vote for the SAME round with a DIFFERENT
    // proposal — both signed by this node. This is what makes an honest peer emit a
    // `ConflictingNotarize`/`ConflictingFinalize` activity. We test the
    // forged-proposal + sign step directly (the p2p broadcast and `Vote::decode`
    // round-trip are commonware library behaviour, not re-pinned here).
    #[test]
    fn equivocates_same_round_different_proposal() {
        let mut rng = StdRng::seed_from_u64(1);
        // 4-validator committee; node 0 signs (pure-multisig, no beacon share).
        let peer_sks: Vec<_> = (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let bls_kps: Vec<_> = (0..4)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<_, _> = peer_sks
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
        let scheme = build_signer(&fluent_namespace(20_994), bimap, &bls_kps[0], None)
            .expect("node 0 is in committee");

        let round = Round::new(Epoch::new(7), View::new(42));
        let parent = View::new(41);
        // The honest proposal this node "received".
        let honest = Proposal::new(round, parent, Digest(B256::repeat_byte(0xaa)));

        // The actor signs the honest proposal AND a forged same-round partner.
        let forged = forged_proposal(round, parent, &mut rng);
        let n_honest = Notarize::<BlsScheme, _>::sign(&scheme, honest).expect("sign honest");
        let n_forged = Notarize::<BlsScheme, _>::sign(&scheme, forged).expect("sign forged");

        // Equivocation invariants an honest peer checks: same round, differing
        // proposals, same parent view.
        assert_eq!(n_honest.round(), n_forged.round(), "same round");
        assert_ne!(
            n_honest.proposal.payload, n_forged.proposal.payload,
            "differing proposals (the equivocation)"
        );
        assert_eq!(
            n_honest.proposal.parent, n_forged.proposal.parent,
            "same parent view"
        );
    }

    // The robustness path the `run()` startup probe + the `broadcast_*` skips rely
    // on: a scheme that CANNOT sign (no secret/share for this epoch — modelled here
    // by a verifier-only scheme, the same `sign() -> None` contract a shareless
    // beacon-active member hits via `CombinedScheme` self-suppression) yields `None`
    // rather than panicking. Without this contract the equivocator would `.unwrap()`
    // and crash, or (pre-fix) silently produce nothing with no warning. The seeded
    // happy path (signer WITH a share equivocates) is covered end-to-end by the
    // `case-byzantine` smoke.
    #[test]
    fn unsignable_scheme_yields_none_not_panic() {
        let mut rng = StdRng::seed_from_u64(2);
        let peer_sks: Vec<_> = (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let bls_kps: Vec<_> = (0..4)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<_, _> = peer_sks
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
        let verifier = build_verifier(&fluent_namespace(20_994), bimap, None);
        let round = Round::new(Epoch::new(7), View::new(42));
        let proposal = Proposal::new(round, View::new(41), Digest(B256::repeat_byte(0xbb)));
        assert!(
            Notarize::<BlsScheme, _>::sign(&verifier, proposal).is_none(),
            "verifier-only scheme must not sign — drives the equivocator's skip+warn path"
        );
    }
}
