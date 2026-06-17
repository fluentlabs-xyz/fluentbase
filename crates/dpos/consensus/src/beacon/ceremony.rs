//! Event-driven DKG ceremony state machine — `run_local_dkg`'s logic decomposed
//! into message handlers so the networked actor can drive one node's dealer +
//! player roles over `BEACON_CHANNEL` (Model B: `committee[E]` deals to itself,
//! dealers == players).
//!
//! Lifecycle (two phases, separated by the height-pinned collection deadline):
//! 1. DEALING — [`DkgCeremony::start`] emits this node's commitment (broadcast) +
//!    one private share per other player (point-to-point); incoming
//!    `Commitment`+`Share` from each dealer are buffered until both are present,
//!    then acked; incoming `Ack`s feed this node's own dealer.
//! 2. At the deadline [`DkgCeremony::seal_dealings`] finalizes this node's dealer
//!    into a signed log (broadcast as `Reveal`, recorded locally); incoming
//!    `Reveal`s are recorded. [`DkgCeremony::finalize`] then derives the agreed
//!    [`Output`] (`PK_E`) + this node's secret [`Share`] over the collected logs.
//!
//! `Player::finalize` is intentionally DEFERRED to [`finalize`] (the boundary),
//! never over a locally-selected `Q` mid-flight.

use crate::beacon::dkg_msg::{DealerCommitment, DkgBody, DkgMsg};
use commonware_cryptography::{
    bls12381::{
        dkg::{observe, Dealer, DealerPrivMsg, Error as DkgError, Info, Logs, Output, Player},
        primitives::{group::Share, sharing::Mode, variant::MinSig},
    },
    ed25519::{self, PrivateKey as Ed25519PrivateKey},
    Signer as _,
};
use commonware_parallel::Sequential;
use commonware_utils::{ordered::Set, N3f1};
use fluentbase_bls::PeerPubkey;
use rand_core::CryptoRngCore;
use std::collections::BTreeMap;

/// The agreed group output of a finished ceremony (`PK_E` + public polynomial).
pub type CeremonyOutput = Output<MinSig, PeerPubkey>;

/// Where an outgoing ceremony message is sent.
#[derive(Clone, Debug)]
pub enum Target {
    /// To every other committee member (commitments, reveals).
    Broadcast,
    /// To one player (a private share, an ack to a dealer).
    Direct(PeerPubkey),
}

/// A ceremony message paired with its delivery target.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub target: Target,
    pub msg: DkgMsg,
}

/// One node's live DKG ceremony for a single epoch.
pub struct DkgCeremony {
    epoch: u64,
    info: Info<MinSig, PeerPubkey>,
    /// This node's dealer (consumed by `seal_dealings`).
    dealer: Option<Dealer<MinSig, Ed25519PrivateKey>>,
    /// This node's player (consumed by `finalize`).
    player: Option<Player<MinSig, Ed25519PrivateKey>>,
    logs: Logs<MinSig, PeerPubkey, N3f1>,
    /// Buffered commitments/shares awaiting their counterpart (a player needs both
    /// a dealer's commitment AND its private share before it can ack).
    pending_pub: BTreeMap<PeerPubkey, DealerCommitment>,
    pending_priv: BTreeMap<PeerPubkey, DealerPrivMsg>,
}

impl DkgCeremony {
    /// Begin a fresh ceremony for `epoch` over `committee` (Model B: dealers ==
    /// players == `committee`). Returns the initial outgoing messages: this node's
    /// commitment (broadcast) + one private share per OTHER player; this node's own
    /// dealing is fed into its own player/dealer locally.
    pub fn start<R: CryptoRngCore>(
        rng: &mut R,
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
    ) -> Result<(Self, Vec<Outgoing>), DkgError> {
        let me = me_key.public_key();
        let info = Info::<MinSig, PeerPubkey>::new::<N3f1>(
            namespace,
            epoch,
            None,
            Mode::NonZeroCounter,
            committee.clone(),
            committee,
        )?;
        let mut player = Player::new(info.clone(), me_key.clone())?;
        let (mut dealer, pub_msg, priv_msgs) =
            Dealer::start::<N3f1>(rng, info.clone(), me_key.clone(), None)?;

        let mut out = vec![Outgoing {
            target: Target::Broadcast,
            msg: DkgMsg {
                ceremony_epoch: epoch,
                body: DkgBody::Commitment(Box::new(pub_msg.clone())),
            },
        }];
        for (pk, priv_msg) in priv_msgs {
            if pk == me {
                // Self-dealing: process our own commitment+share locally so our
                // player counts it and our dealer collects our self-ack.
                if let Some(ack) =
                    player.dealer_message::<N3f1>(me.clone(), pub_msg.clone(), priv_msg)
                {
                    let _ = dealer.receive_player_ack(me.clone(), ack);
                }
            } else {
                out.push(Outgoing {
                    target: Target::Direct(pk),
                    msg: DkgMsg {
                        ceremony_epoch: epoch,
                        body: DkgBody::Share(priv_msg),
                    },
                });
            }
        }

        let logs = Logs::<MinSig, PeerPubkey, N3f1>::new(info.clone());
        Ok((
            Self {
                epoch,
                info,
                dealer: Some(dealer),
                player: Some(player),
                logs,
                pending_pub: BTreeMap::new(),
                pending_priv: BTreeMap::new(),
            },
            out,
        ))
    }

    /// Handle one incoming ceremony message from peer `from`. Invalid messages are
    /// dropped (the commonware primitives validate internally); returns any
    /// outgoing messages this triggers (an ack on a complete dealing).
    pub fn handle(&mut self, from: PeerPubkey, body: DkgBody) -> Vec<Outgoing> {
        match body {
            DkgBody::Commitment(pub_msg) => {
                self.pending_pub.insert(from.clone(), *pub_msg);
                self.try_ack(from)
            }
            DkgBody::Share(priv_msg) => {
                self.pending_priv.insert(from.clone(), priv_msg);
                self.try_ack(from)
            }
            DkgBody::Ack(ack) => {
                if let Some(dealer) = self.dealer.as_mut() {
                    let _ = dealer.receive_player_ack(from, ack);
                }
                Vec::new()
            }
            DkgBody::Reveal(signed) => {
                if let Some((pk, log)) = (*signed).check(&self.info) {
                    self.logs.record(pk, log);
                }
                Vec::new()
            }
        }
    }

    /// If both the commitment and private share from `dealer_pk` are buffered,
    /// process the dealing and emit an ack (point-to-point) if the player accepts.
    fn try_ack(&mut self, dealer_pk: PeerPubkey) -> Vec<Outgoing> {
        if !(self.pending_pub.contains_key(&dealer_pk) && self.pending_priv.contains_key(&dealer_pk))
        {
            return Vec::new();
        }
        let pub_msg = self.pending_pub.remove(&dealer_pk).expect("present");
        let priv_msg = self.pending_priv.remove(&dealer_pk).expect("present");
        let Some(player) = self.player.as_mut() else {
            return Vec::new();
        };
        match player.dealer_message::<N3f1>(dealer_pk.clone(), pub_msg, priv_msg) {
            Some(ack) => vec![Outgoing {
                target: Target::Direct(dealer_pk),
                msg: DkgMsg {
                    ceremony_epoch: self.epoch,
                    body: DkgBody::Ack(ack),
                },
            }],
            None => Vec::new(),
        }
    }

    /// Close the dealing phase: finalize this node's dealer into a signed log,
    /// record it locally, and broadcast it as a `Reveal` so every player can
    /// include it. Idempotent — a second call is a no-op (dealer already taken).
    pub fn seal_dealings(&mut self) -> Vec<Outgoing> {
        let Some(dealer) = self.dealer.take() else {
            return Vec::new();
        };
        let signed = dealer.finalize::<N3f1>();
        if let Some((pk, log)) = signed.clone().check(&self.info) {
            self.logs.record(pk, log);
        }
        vec![Outgoing {
            target: Target::Broadcast,
            msg: DkgMsg {
                ceremony_epoch: self.epoch,
                body: DkgBody::Reveal(Box::new(signed)),
            },
        }]
    }

    /// Non-destructively probe whether the ceremony can now derive its agreed
    /// output — i.e. a quorum of valid dealer logs is selectable. Uses `observe`
    /// over a CLONE of the collected logs (`Logs` is `Clone`; `Player` is not),
    /// so the ceremony is left intact: the supervisor calls this each tick after
    /// [`seal_dealings`](Self::seal_dealings) until it returns `true`, THEN
    /// [`finalize`](Self::finalize). A `true` means a subsequent `finalize` will
    /// select the same quorum and succeed — the share can be memoized before the
    /// epoch boundary block is proposed/verified.
    pub fn ready<R: CryptoRngCore>(&self, rng: &mut R) -> bool {
        observe::<MinSig, PeerPubkey, N3f1, ed25519::Batch>(rng, self.logs.clone(), &Sequential)
            .is_ok()
    }

    /// Derive the agreed [`CeremonyOutput`] (`PK_E`) + this node's secret [`Share`]
    /// over the collected logs.
    ///
    /// CALLER CONTRACT (the supervisor enforces timing; this is destructive):
    /// - [`seal_dealings`](Self::seal_dealings) MUST have run first — it is the only
    ///   place this node's own dealer log enters `self.logs` and is broadcast, so
    ///   finalizing without sealing drops this dealer from the quorum (locally AND
    ///   for every peer).
    /// - At least a dealer-quorum of VALID logs must be recorded, else
    ///   `Player::finalize` returns `Err(DkgFailed)`. Because this CONSUMES the
    ///   ceremony, a premature/under-quorum call is irrecoverable (the whole epoch's
    ///   ceremony is lost) — the supervisor must only call it once the collection
    ///   window has closed AND the logs are agreed (the option-A stall is a HALT the
    ///   supervisor resumes by retrying delivery + finalize, NOT by dropping state).
    pub fn finalize<R: CryptoRngCore>(
        self,
        rng: &mut R,
    ) -> Result<(CeremonyOutput, Share), DkgError> {
        let player = self.player.expect("player present until finalize");
        player.finalize::<N3f1, ed25519::Batch>(rng, self.logs, &Sequential)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::outcome::group_public_key;
    use commonware_consensus::types::{Epoch, Round, View};
    use commonware_math::algebra::Random as _;
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial, verify_seed};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// Drive N ceremonies to completion purely through the event API (start →
    /// exchange Outgoing → seal → exchange → finalize), then assert every node
    /// agreed on `PK_E` and that the resulting shares recover a verifiable seed —
    /// i.e. the networked event model reproduces `run_local_dkg`.
    #[test]
    fn ceremonies_agree_and_feed_a_verifiable_seed_via_events() {
        let mut rng = StdRng::seed_from_u64(11);
        let keys: Vec<Ed25519PrivateKey> =
            (0..5).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        // (sender, outgoing) — the sender is the `from` the receiver's handle needs.
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, out) =
                DkgCeremony::start(&mut rng, ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(out.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }

        // Deliver until quiescent (commitments/shares → acks; acks → nothing).
        deliver_all(&mut ceremonies, &mut queue);

        // Deadline: every node seals its dealings (broadcast Reveal); deliver them.
        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let out = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(out.into_iter().map(|o| (pk.clone(), o)));
        }
        deliver_all(&mut ceremonies, &mut queue);

        // Finalize: every node derives the agreed output + its share.
        let mut outputs = Vec::new();
        let mut shares = BTreeMap::new();
        for (pk, cer) in ceremonies {
            let (out, share) = cer.finalize(&mut rng).expect("finalize");
            shares.insert(pk, share);
            outputs.push(out);
        }
        assert_eq!(shares.len(), keys.len(), "every node derives a share");
        let pk0 = group_public_key(&outputs[0]);
        for o in &outputs[1..] {
            assert_eq!(group_public_key(o), pk0, "all nodes agree on PK_E");
        }

        // The DKG shares recover a seed that verifies against PK_E.
        let seed_ns = seed_namespace(ns);
        let round = Round::new(Epoch::new(0), View::new(100));
        let partials: Vec<_> = shares
            .values()
            .map(|s| sign_seed_partial(s, &seed_ns, round))
            .collect();
        let sig = recover_seed(outputs[0].public(), &partials).expect("recover");
        assert!(
            verify_seed(pk0, &seed_ns, round, &sig),
            "seed from the event-driven DKG shares must verify against PK_E"
        );
    }

    fn deliver_all(
        ceremonies: &mut BTreeMap<PeerPubkey, DkgCeremony>,
        queue: &mut Vec<(PeerPubkey, Outgoing)>,
    ) {
        while let Some((from, o)) = queue.pop() {
            match o.target {
                Target::Broadcast => {
                    let recipients: Vec<PeerPubkey> =
                        ceremonies.keys().filter(|pk| **pk != from).cloned().collect();
                    for to in recipients {
                        let more = ceremonies
                            .get_mut(&to)
                            .expect("ceremony")
                            .handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.into_iter().map(|m| (to.clone(), m)));
                    }
                }
                Target::Direct(to) => {
                    if let Some(cer) = ceremonies.get_mut(&to) {
                        let more = cer.handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            }
        }
    }
}
