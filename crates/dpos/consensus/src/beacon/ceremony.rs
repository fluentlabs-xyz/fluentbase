//! Event-driven DKG ceremony state machine: the dealer and player roles one node
//! runs over `BEACON_CHANNEL` (Model B: `committee[E]` deals to itself, dealers
//! == players).
//!
//! Dealing emits this node's commitment and one private share per other player;
//! both legs are reliable — the dealer re-sends un-acked dealings every pre-seal
//! tick, and a player re-emits its cached ack on any re-receipt.
//!
//! At the deadline the dealer is finalized into a signed log and broadcast as a
//! `Reveal`. The agreed `Output` and this node's share are then derived over
//! exactly the dealer-log set the epoch-key agreement certified.
//!
//! [`Player::finalize`] waits for the agreed pinned set to be fully held and
//! quorum-ready, never a locally-selected quorum mid-flight.

use crate::beacon::dkg_agree::PinnedDerive;
use crate::beacon::dkg_msg::{Ack, DealerCommitment, DealerReveal, DkgBody, DkgMsg};
use crate::beacon::share_state::JournalRecord;
use alloy_primitives::{keccak256, B256};
use commonware_codec::Encode as _;
use commonware_cryptography::{
    bls12381::{
        dkg::{
            observe, Dealer, DealerLog, DealerPrivMsg, Error as DkgError, Info, Logs, Output,
            Player,
        },
        primitives::{group::Share, sharing::Mode, variant::MinSig},
    },
    ed25519::{self, PrivateKey as Ed25519PrivateKey},
    transcript::Transcript,
    Signer as _,
};
use commonware_parallel::Sequential;
use commonware_utils::{ordered::Set, N3f1};
use fluentbase_bls::PeerPubkey;
use rand_core::CryptoRngCore;
use std::collections::BTreeMap;

/// The agreed group output of a finished ceremony (`PK_E` + public polynomial).
pub(crate) type CeremonyOutput = Output<MinSig, PeerPubkey>;

/// The identity of one recorded dealer log: the dealer that signed it and the
/// content hash of the signed bytes ([`log_hash`]). A dealer is not a log — a
/// Byzantine dealer can sign two valid logs over the same `Info` — so keying by
/// `(dealer, hash)` lets the ceremony hold both and name which one the agreement
/// pinned.
pub(crate) type LogId = (PeerPubkey, B256);

/// The content hash of a signed dealer log, `keccak256(encode(SignedDealerLog))`.
/// Shared by the resolver key, the published index and the agreement's pinned
/// set, so a log is never compared under two different functions.
pub(crate) fn log_hash(signed: &DealerReveal) -> B256 {
    keccak256(signed.encode())
}

/// The two valid logs one dealer signed for one epoch, as their content hashes
/// (the bodies stay in the recorded set under their own [`LogId`]s). `first` is
/// the hash this node recorded first, which is the one it proposes and confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DealerEquivocation {
    pub first: B256,
    pub second: B256,
}

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
pub(crate) struct Outgoing {
    pub target: Target,
    pub msg: DkgMsg,
}

/// What one ceremony step produced: outgoing messages and the durable
/// [`JournalRecord`]s the actor appends so a restart can resume.
#[derive(Default)]
pub(crate) struct Step {
    pub outgoing: Vec<Outgoing>,
    pub journal: Vec<JournalRecord>,
    /// The log this step newly recorded, with its content hash. The dealer is the
    /// log's signer, not the sender, so a relayed valid `Reveal` is recorded under
    /// that dealer. The actor needs the id to name which claim a failed journal
    /// append left unbacked.
    pub recorded_log: Option<LogId>,
    /// A dealer this step proved equivocating: the recorded log is its second valid
    /// log for this epoch under a different hash. Set once per `(epoch, dealer)`.
    pub equivocation: Option<PeerPubkey>,
}

impl Step {
    fn out(outgoing: Vec<Outgoing>) -> Self {
        Self {
            outgoing,
            journal: Vec::new(),
            recorded_log: None,
            equivocation: None,
        }
    }

    /// Whether this step recorded a new dealer log into the finalizable set, the only
    /// journal records that can change finalizability, so the actor can skip
    /// `drive_finalization` after an ack-only step.
    pub fn recorded_a_log(&self) -> bool {
        self.journal.iter().any(|r| {
            matches!(
                r,
                JournalRecord::OwnSeal(_)
                    | JournalRecord::PeerLog(_)
                    | JournalRecord::DealerEquivocation(..)
            )
        })
    }
}

/// What [`DkgCeremony::insert_log`] found when a `check`-valid log landed.
enum Inserted {
    /// The `(dealer, hash)` was already held — nothing changed.
    Duplicate,
    /// The dealer's first log this epoch.
    First,
    /// The dealer's second log under a different hash: the equivocation pair was
    /// created now, `first` being the log recorded before this one.
    Equivocation { first: Box<DealerReveal> },
    /// A further log of a dealer already proven equivocating (reachable only through
    /// a targeted pinned fetch, or a journal replay of one).
    Another,
}

/// A ceremony reconstructed from its journal by [`DkgCeremony::resume`]: the
/// rebuilt ceremony and the messages to re-emit. Seal state is not a separate
/// flag — the actor derives it from the ceremony
/// ([`DkgCeremony::dealing_closed`] and [`DkgCeremony::own_log_recorded`]).
///
/// Resume has two shapes, chosen by the caller's timing gate:
/// - pre-seal: the dealer is re-derived from [`dealer_seed_rng`], reproducing the
///   byte-identical commitment and shares, and both delivery legs resume;
/// - at or after the deadline: the dealer role is retired and never re-seals,
///   because a torn journal cannot prove the node did not already seal and
///   broadcast a divergent log. A journaled own log is re-broadcast verbatim.
pub(crate) struct Resumed {
    pub ceremony: DkgCeremony,
    pub outgoing: Vec<Outgoing>,
}

/// One node's live DKG ceremony for a single epoch.
pub(crate) struct DkgCeremony {
    epoch: u64,
    info: Info<MinSig, PeerPubkey>,
    /// `committee[epoch]`, the set `info` was built over (dealers == players). Kept
    /// beside `info` because commonware's `Info` exposes neither set, and the actor
    /// asks [`has_seat`](Self::has_seat) before handing a frame in.
    roster: Set<PeerPubkey>,
    /// This node's dealer (consumed by `seal_dealings`).
    dealer: Option<Dealer<MinSig, Ed25519PrivateKey>>,
    /// This node's player (consumed by `finalize`).
    player: Option<Player<MinSig, Ed25519PrivateKey>>,
    /// Buffered commitments/shares awaiting their counterpart (a player needs both
    /// a dealer's commitment and its private share before it can ack).
    pending_pub: BTreeMap<PeerPubkey, DealerCommitment>,
    pending_priv: BTreeMap<PeerPubkey, DealerPrivMsg>,
    /// Every valid signed log this ceremony holds, by [`LogId`]. The signed form is
    /// what the actor serves and re-broadcasts, and what the pinned-set finalize
    /// selects from by exact `(dealer, hash)`. One dealer may own more than one
    /// entry: that is an equivocation, never a replacement.
    signed_logs: BTreeMap<LogId, DealerReveal>,
    /// The hash this node recorded first for each dealer, which is what it publishes
    /// into the shared index, proposes and confirms. Stable for the ceremony's life.
    first_log: BTreeMap<PeerPubkey, B256>,
    /// Dealers proven to have signed two distinct valid logs this epoch, with the
    /// pair. A dealer in here is locally banned from gossip: a further `Reveal` from
    /// it is dropped by [`handle`](Self::handle), while a targeted fetch by exact
    /// hash is still honoured.
    equivocations: BTreeMap<PeerPubkey, DealerEquivocation>,
    /// Our own dealer commitment, retained so [`retransmit`](Self::retransmit) can
    /// re-send it to any player still in `unsent`. `None` on a player-only resume.
    own_pub_msg: Option<DealerCommitment>,
    /// Players who have not yet acked our dealing, holding the private share we owe
    /// each. Pruned as acks land, so retransmit is bounded by it.
    unsent: BTreeMap<PeerPubkey, DealerPrivMsg>,
    /// The ack we emitted as a player, keyed by dealer. On a re-receipt of a dealing
    /// we already acked, [`try_ack`](Self::try_ack) re-emits this cached ack instead
    /// of dropping it, until it lands.
    emitted_acks: BTreeMap<PeerPubkey, Ack>,
}

/// Build the commonware DKG [`Info`] for `epoch` over `committee`: the one place
/// the ceremony's fixed parameters (`N3f1`, `Mode::NonZeroCounter`, dealers ==
/// players) are pinned, so `start`, `resume` and the log re-check cannot drift.
pub(crate) fn info_for(
    namespace: &[u8],
    epoch: u64,
    committee: Set<PeerPubkey>,
) -> Result<Info<MinSig, PeerPubkey>, DkgError> {
    Info::<MinSig, PeerPubkey>::new::<N3f1>(
        namespace,
        epoch,
        None,
        Mode::NonZeroCounter,
        committee.clone(),
        committee,
    )
}

/// The signed logs one journal record carries: none for a dealing or an ack, one
/// for an `OwnSeal`/`PeerLog`, and both halves of a `DealerEquivocation` in the
/// order the live path recorded them.
fn logs_in(record: JournalRecord) -> Vec<DealerReveal> {
    match record {
        JournalRecord::OwnSeal(signed) | JournalRecord::PeerLog(signed) => vec![*signed],
        JournalRecord::DealerEquivocation(first, second) => vec![*first, *second],
        JournalRecord::ReceivedDealing(..) | JournalRecord::OwnDealerAck(..) => Vec::new(),
    }
}

/// The valid logs one journal record evidences, each with the dealer it checked
/// as. A `DealerEquivocation` is taken whole or not at all: both halves must
/// check as the same dealer under different hashes, else neither body is taken
/// from it.
fn checked_logs_in(
    record: JournalRecord,
    info: &Info<MinSig, PeerPubkey>,
    epoch: u64,
) -> Vec<(PeerPubkey, DealerLog<MinSig, PeerPubkey>, DealerReveal)> {
    let is_pair = matches!(record, JournalRecord::DealerEquivocation(..));
    let checked: Vec<_> = logs_in(record)
        .into_iter()
        .filter_map(|signed| {
            signed
                .clone()
                .check(info)
                .map(|(pk, log)| (pk, log, signed))
        })
        .collect();
    if is_pair
        && !matches!(&checked[..], [(pa, _, a), (pb, _, b)] if pa == pb && log_hash(a) != log_hash(b))
    {
        tracing::warn!(
            epoch,
            "live DKG: journaled equivocation record does not evidence a pair \
             (halves fail check, name two dealers, or are one body) — dropped"
        );
        return Vec::new();
    }
    checked
}

/// Re-check a finalized epoch's journaled logs into a `(dealer, hash)` serve map,
/// the cold-load path the log store takes on a miss after a restart.
///
/// Each log is checked against the epoch's `Info` under the same rule the resume
/// applies, so a body the resumed ceremony would not hold is never served. A bad
/// committee or namespace surfaces as `Err` so the caller declines to serve.
pub(crate) fn checked_serve_map(
    namespace: &[u8],
    epoch: u64,
    committee: Set<PeerPubkey>,
    records: Vec<JournalRecord>,
) -> Result<BTreeMap<LogId, DealerReveal>, DkgError> {
    let info = info_for(namespace, epoch, committee)?;
    let mut map = BTreeMap::new();
    for (pk, _, signed) in records
        .into_iter()
        .flat_map(|record| checked_logs_in(record, &info, epoch))
    {
        map.insert((pk, log_hash(&signed)), signed);
    }
    Ok(map)
}

/// The namespace domain-separating the dealer-polynomial seed derivation.
///
/// The intermediate signature this seeds from determines the dealer polynomial,
/// hence the whole share, so it must never be logged or transmitted.
const DEALER_SEED_NS: &[u8] = b"FLUENT_DPOS_DKG_DEALER_SEED_V1";

/// Deterministic dealer-polynomial RNG derived from the validator key and epoch,
/// so re-creating the dealer after a restart reproduces the byte-identical
/// commitment and shares. Ed25519 signing is deterministic, so the seed is stable
/// per `(me_key, epoch)`; it needs the secret key and is never persisted, so it
/// survives datadir loss.
fn dealer_seed_rng(me_key: &Ed25519PrivateKey, epoch: u64) -> impl CryptoRngCore {
    let sig = me_key.sign(DEALER_SEED_NS, &epoch.to_be_bytes());
    let mut t = Transcript::new(DEALER_SEED_NS);
    t.commit(sig.as_ref());
    Transcript::resume(t.summarize()).noise(b"dealer-rng")
}

/// The dealer setup that [`DkgCeremony::start`] and the pre-seal branch of
/// [`DkgCeremony::resume`] run identically. Splits the private shares into our
/// own (returned for the self-ack and journal) and the per-player retransmit set.
/// The self-ack is fed by the caller because its source differs.
#[allow(clippy::type_complexity)]
fn init_dealer(
    me_key: &Ed25519PrivateKey,
    epoch: u64,
    info: &Info<MinSig, PeerPubkey>,
) -> Result<
    (
        Dealer<MinSig, Ed25519PrivateKey>,
        DealerCommitment,
        BTreeMap<PeerPubkey, DealerPrivMsg>,
        DealerPrivMsg,
    ),
    DkgError,
> {
    let me = me_key.public_key();
    let (dealer, pub_msg, priv_msgs) = Dealer::start::<N3f1>(
        dealer_seed_rng(me_key, epoch),
        info.clone(),
        me_key.clone(),
        None,
    )?;
    let mut unsent = BTreeMap::new();
    let mut self_priv: Option<DealerPrivMsg> = None;
    for (pk, priv_msg) in priv_msgs {
        if pk == me {
            self_priv = Some(priv_msg);
        } else {
            unsent.insert(pk, priv_msg);
        }
    }
    let self_priv = self_priv.expect("Model B: me is always a player, so a self-share exists");
    Ok((dealer, pub_msg, unsent, self_priv))
}

impl DkgCeremony {
    /// Begin a fresh ceremony for `epoch` over `committee`. The dealer polynomial is
    /// derived deterministically from the validator key and epoch ([`dealer_seed_rng`]),
    /// so a restart re-derives the identical commitment and shares.
    ///
    /// Returns this node's commitment (broadcast), one private share per other player,
    /// and the journal record of our own self-dealing.
    pub fn start(
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
    ) -> Result<(Self, Step), DkgError> {
        let me = me_key.public_key();
        let info = info_for(namespace, epoch, committee.clone())?;
        let mut player = Player::new(info.clone(), me_key.clone())?;
        let (mut dealer, pub_msg, unsent, self_priv) = init_dealer(&me_key, epoch, &info)?;

        // Self-dealing: process our own commitment and share locally so our player counts
        // it and our dealer collects its own self-ack, else the dealer reveals its own
        // point at seal and burns an f-slot.
        let mut emitted_acks: BTreeMap<PeerPubkey, Ack> = BTreeMap::new();
        if let Some(ack) =
            player.dealer_message::<N3f1>(me.clone(), pub_msg.clone(), self_priv.clone())
        {
            let _ = dealer.receive_player_ack(me.clone(), ack.clone());
            emitted_acks.insert(me.clone(), ack);
        }

        let mut step = Step::out(vec![Outgoing {
            target: Target::Broadcast,
            msg: DkgMsg {
                ceremony_epoch: epoch,
                body: DkgBody::Commitment(Box::new(pub_msg.clone())),
            },
        }]);
        step.journal.push(JournalRecord::ReceivedDealing(
            me.clone(),
            Box::new(pub_msg.clone()),
            Box::new(self_priv),
        ));
        for (pk, priv_msg) in &unsent {
            step.outgoing.push(Outgoing {
                target: Target::Direct(pk.clone()),
                msg: DkgMsg {
                    ceremony_epoch: epoch,
                    body: DkgBody::Share(priv_msg.clone()),
                },
            });
        }

        Ok((
            Self {
                epoch,
                info,
                roster: committee,
                dealer: Some(dealer),
                player: Some(player),
                pending_pub: BTreeMap::new(),
                pending_priv: BTreeMap::new(),
                signed_logs: BTreeMap::new(),
                first_log: BTreeMap::new(),
                equivocations: BTreeMap::new(),
                own_pub_msg: Some(pub_msg),
                unsent,
                emitted_acks,
            },
            step,
        ))
    }

    /// Handle one incoming ceremony message from peer `from`. Invalid messages are
    /// dropped (the commonware primitives validate internally); returns the outgoing
    /// messages this triggers (an ack on a complete dealing) and the journal records
    /// the actor must persist (the accepted dealing, or a peer's recorded log).
    pub fn handle(&mut self, from: PeerPubkey, body: DkgBody) -> Step {
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
                let mut step = Step::default();
                if let Some(dealer) = self.dealer.as_mut() {
                    let _ = dealer.receive_player_ack(from.clone(), ack.clone());
                }
                // Prune the acking player from the retransmit set and journal the ack so a
                // pre-seal resume can replay it. A wrong-signature ack still prunes: it only
                // self-harms by at most f.
                if self.unsent.remove(&from).is_some() {
                    step.journal
                        .push(JournalRecord::OwnDealerAck(from, Box::new(ack)));
                }
                step
            }
            // Gossip ingress, and the one place the local ban applies: a dealer already
            // proven equivocating records no further log from gossip. Its pinned log still
            // arrives through a targeted fetch by exact hash.
            DkgBody::Reveal(signed) => match (*signed).clone().check(&self.info) {
                Some((pk, _)) if self.equivocations.contains_key(&pk) => Step::default(),
                Some((pk, _)) => self.record_checked_log(pk, *signed),
                None => Step::default(),
            },
            // A share-confirmation is not ceremony traffic: the actor intercepts it before
            // this dispatch. Present so the match stays exhaustive.
            DkgBody::Confirm(_) => Step::default(),
        }
    }

    /// The recording rule shared by every path a valid log enters on — our own seal, a
    /// gossip `Reveal`, a resolver delivery, a journal replay — so all key the log
    /// identically. Inserts `signed` under its [`LogId`] and says what that made it.
    ///
    /// The ban is not applied here: it is a property of gossip ingress, and a targeted
    /// fetch or a replay must record what gossip would refuse.
    fn insert_log(&mut self, pk: PeerPubkey, signed: DealerReveal) -> Inserted {
        let hash = log_hash(&signed);
        if self.signed_logs.contains_key(&(pk.clone(), hash)) {
            return Inserted::Duplicate;
        }
        let inserted = match self.first_log.get(&pk) {
            None => {
                self.first_log.insert(pk.clone(), hash);
                Inserted::First
            }
            Some(first) => match self.signed_logs.get(&(pk.clone(), *first)) {
                Some(first_signed) if !self.equivocations.contains_key(&pk) => {
                    self.equivocations.insert(
                        pk.clone(),
                        DealerEquivocation {
                            first: *first,
                            second: hash,
                        },
                    );
                    Inserted::Equivocation {
                        first: Box::new(first_signed.clone()),
                    }
                }
                _ => Inserted::Another,
            },
        };
        self.signed_logs.insert((pk, hash), signed);
        inserted
    }

    /// Record a peer's valid log, returning the journal record that backs it and the
    /// [`LogId`] it went in under. An empty step on a duplicate bounds journal growth.
    ///
    /// A dealer's second log under a different hash is journaled as one
    /// [`JournalRecord::DealerEquivocation`] carrying the pair, so the record that
    /// restores the ban after a restart is the record that restores the body.
    fn record_checked_log(&mut self, pk: PeerPubkey, signed: DealerReveal) -> Step {
        let hash = log_hash(&signed);
        let mut step = Step::default();
        match self.insert_log(pk.clone(), signed.clone()) {
            Inserted::Duplicate => return step,
            Inserted::First | Inserted::Another => {
                step.journal.push(JournalRecord::PeerLog(Box::new(signed)));
            }
            Inserted::Equivocation { first } => {
                step.journal
                    .push(JournalRecord::DealerEquivocation(first, Box::new(signed)));
                step.equivocation = Some(pk.clone());
            }
        }
        step.recorded_log = Some((pk, hash));
        step
    }

    /// The journal record that backs one recorded log — what the actor re-appends
    /// when the original write did not land: the pair record for the second half of
    /// an equivocation, a `PeerLog` for anything else. `None` if the id is not held.
    pub fn journal_record_for(&self, id: &LogId) -> Option<JournalRecord> {
        let signed = self.signed_logs.get(id)?.clone();
        match self.equivocations.get(&id.0) {
            Some(pair) if pair.second == id.1 => {
                let first = self.signed_logs.get(&(id.0.clone(), pair.first))?.clone();
                Some(JournalRecord::DealerEquivocation(
                    Box::new(first),
                    Box::new(signed),
                ))
            }
            _ => Some(JournalRecord::PeerLog(Box::new(signed))),
        }
    }

    /// Process the buffered dealing from `dealer_pk` once both halves are present and
    /// emit an ack if the player accepts. On accept, journal the dealing and cache the
    /// ack. On a re-receipt, re-emit the cached ack instead of dropping it.
    fn try_ack(&mut self, dealer_pk: PeerPubkey) -> Step {
        if !(self.pending_pub.contains_key(&dealer_pk)
            && self.pending_priv.contains_key(&dealer_pk))
        {
            return Step::default();
        }
        let pub_msg = self.pending_pub.remove(&dealer_pk).expect("present");
        let priv_msg = self.pending_priv.remove(&dealer_pk).expect("present");
        let Some(player) = self.player.as_mut() else {
            return Step::default();
        };
        match player.dealer_message::<N3f1>(dealer_pk.clone(), pub_msg.clone(), priv_msg.clone()) {
            Some(ack) => {
                self.emitted_acks.insert(dealer_pk.clone(), ack.clone());
                let mut step = Step::out(vec![Outgoing {
                    target: Target::Direct(dealer_pk.clone()),
                    msg: DkgMsg {
                        ceremony_epoch: self.epoch,
                        body: DkgBody::Ack(ack),
                    },
                }]);
                step.journal.push(JournalRecord::ReceivedDealing(
                    dealer_pk,
                    Box::new(pub_msg),
                    Box::new(priv_msg),
                ));
                step
            }
            None => match self.emitted_acks.get(&dealer_pk) {
                Some(ack) => Step::out(vec![Outgoing {
                    target: Target::Direct(dealer_pk),
                    msg: DkgMsg {
                        ceremony_epoch: self.epoch,
                        body: DkgBody::Ack(ack.clone()),
                    },
                }]),
                None => Step::default(),
            },
        }
    }

    /// Forget the ack this node emitted as a player for `dealer`'s dealing: the
    /// actor's durability gate found the paired `ReceivedDealing` not durable and
    /// withheld the ack. Without this the next retransmit would re-emit an ack with an
    /// empty journal, which the gate reads as already durable.
    pub fn withhold_ack(&mut self, dealer: &PeerPubkey) {
        self.emitted_acks.remove(dealer);
    }

    /// Re-send each un-acked dealing (`Commitment` and `Share`) point-to-point while
    /// the dealing phase is open. `unsent` shrinks as acks land, so this converges in a
    /// tick or two; it is a no-op once sealed.
    pub fn retransmit(&self) -> Vec<Outgoing> {
        if self.dealing_closed() {
            return Vec::new();
        }
        let Some(pub_msg) = self.own_pub_msg.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(self.unsent.len() * 2);
        for (pk, priv_msg) in &self.unsent {
            out.push(Outgoing {
                target: Target::Direct(pk.clone()),
                msg: DkgMsg {
                    ceremony_epoch: self.epoch,
                    body: DkgBody::Commitment(Box::new(pub_msg.clone())),
                },
            });
            out.push(Outgoing {
                target: Target::Direct(pk.clone()),
                msg: DkgMsg {
                    ceremony_epoch: self.epoch,
                    body: DkgBody::Share(priv_msg.clone()),
                },
            });
        }
        out
    }

    /// Close the dealing phase: finalize this node's dealer into a signed log, record
    /// it locally, and broadcast it as a `Reveal`. Idempotent.
    ///
    /// The `OwnSeal` record and the `Reveal` are produced only when our freshly
    /// finalized log checks against our own `Info`: journaling an `OwnSeal` whose log
    /// is absent would let a resume mark the epoch sealed with no log to re-broadcast,
    /// and broadcasting an uncheckable `Reveal` only spends bandwidth.
    pub fn seal_dealings(&mut self) -> Step {
        let Some(dealer) = self.dealer.take() else {
            return Step::default();
        };
        let signed = dealer.finalize::<N3f1>();
        let mut step = Step::default();
        if let Some((pk, _)) = signed.clone().check(&self.info) {
            // Our own log goes in under the same rule as every other, and it can only be the
            // first log of `me`: the dealer is taken once, nobody holds our log before this
            // broadcast, and the seeded dealer makes a second hash impossible. Any other
            // outcome is a broken invariant, reported below.
            let inserted = self.insert_log(pk, signed.clone());
            debug_assert!(
                matches!(inserted, Inserted::First),
                "own sealed log is not the first log of `me` — the seeded dealer self-equivocated"
            );
            if !matches!(inserted, Inserted::First) {
                tracing::error!(
                    epoch = self.epoch,
                    "live DKG: own sealed log was not the first log recorded for this node — \
                     a second body of the seeded dealer exists; invariant broken"
                );
            }
            step.journal
                .push(JournalRecord::OwnSeal(Box::new(signed.clone())));
            step.outgoing.push(Outgoing {
                target: Target::Broadcast,
                msg: DkgMsg {
                    ceremony_epoch: self.epoch,
                    body: DkgBody::Reveal(Box::new(signed)),
                },
            });
            tracing::info!(
                epoch = self.epoch,
                "live DKG: dealings sealed — own log broadcast"
            );
        } else {
            // Our own freshly-finalized log failed self-check. Emitting no `OwnSeal`/`Reveal`
            // is correct, but otherwise silent — surface it so the node does not quietly
            // self-remove from the dealer quorum with no diagnostic.
            tracing::warn!(
                epoch = self.epoch,
                "live DKG: own sealed dealer log failed self-check — no OwnSeal/Reveal emitted; \
                 this node contributes no dealing this epoch (recovers its share as a player)"
            );
        }
        step
    }

    /// The number of distinct `(dealer, hash)` logs recorded so far. An equivocating
    /// dealer counts once per log, so this counts bodies held, not dealers.
    #[cfg(test)]
    pub fn recorded_log_count(&self) -> usize {
        self.signed_logs.len()
    }

    /// Whether this ceremony holds the log `id` names — the exact body, by content
    /// hash. What the pinned-set fetch asks before requesting a body: a dealer whose
    /// held log is a different body than the pinned one is `false` here and gets
    /// re-fetched by hash, where a per-dealer check would have called it held.
    pub fn holds(&self, id: &LogId) -> bool {
        self.signed_logs.contains_key(id)
    }

    /// The evidence that `dealer` signed two distinct logs this epoch, if it did:
    /// the pair of content hashes, both bodies being held under their [`LogId`]s.
    pub fn equivocation(&self, dealer: &PeerPubkey) -> Option<&DealerEquivocation> {
        self.equivocations.get(dealer)
    }

    /// Every proven equivocation this ceremony holds — what the actor copies out on a
    /// resume, since its own map is the one that outlives the ceremony.
    pub fn equivocations(&self) -> &BTreeMap<PeerPubkey, DealerEquivocation> {
        &self.equivocations
    }

    /// The dealing phase is closed: we sealed, or check-failed, or resumed player-only.
    /// The dealer `Option` is the durable in-ceremony seal state, taken before its
    /// check, so the actor derives seal suppression from it rather than a flag.
    pub fn dealing_closed(&self) -> bool {
        self.dealer.is_none()
    }

    /// Whether `peer` holds a seat in this ceremony, which under Model B is both "one
    /// of its dealers" and "one of its players". The actor asks this before handing a
    /// frame in, because commonware answers a stranger silently.
    pub fn has_seat(&self, peer: &PeerPubkey) -> bool {
        self.roster.position(peer).is_some()
    }

    /// Our own valid log is recorded: the seal-before-finalize precondition and the
    /// torn-own-seal recovery target. A check-failed seal correctly does not finalize
    /// over a set missing its own valid log.
    pub fn own_log_recorded(&self, me: &PeerPubkey) -> bool {
        self.first_log.contains_key(me)
    }

    /// The signed form of one recorded dealer log, by exact [`LogId`], served to a
    /// peer over the DKG-log recovery resolver. Every entry is check-valid by
    /// construction, so a served log always re-verifies on the requester.
    pub fn signed_log(&self, id: &LogId) -> Option<&DealerReveal> {
        self.signed_logs.get(id)
    }

    /// Take this ceremony's recorded signed logs, leaving it empty. The actor calls
    /// this before the finalize to seed the finalized epoch's entry in the serve store,
    /// so the recovery producer can keep serving them without a disk read or per-request
    /// check.
    pub fn take_signed_logs(&mut self) -> BTreeMap<LogId, DealerReveal> {
        std::mem::take(&mut self.signed_logs)
    }

    /// Re-check and record one signed log recovered via the resolver. The log must
    /// check-verify, be signed by `expected.0` and hash to `expected.1`; a valid log
    /// for a different dealer or of the other body does not satisfy the fetch.
    ///
    /// Returns whether the log was accepted, and the step carrying the journal record.
    /// The local ban does not apply here: this is how the agreed log of a banned dealer
    /// arrives.
    pub fn ingest_signed_log(&mut self, expected: &LogId, signed: DealerReveal) -> (bool, Step) {
        match signed.clone().check(&self.info) {
            Some((pk, _)) if pk != expected.0 || log_hash(&signed) != expected.1 => {
                // A valid log, but not the one fetched — do not record it under
                // this fetch; the resolver re-fetches `expected` elsewhere.
                (false, Step::default())
            }
            Some((pk, _)) => {
                // Valid + exactly the body asked for: record (deduped) and accept. An
                // honest duplicate (already held) returns an empty step — valid, no
                // journal.
                (true, self.record_checked_log(pk, signed))
            }
            None => (false, Step::default()),
        }
    }

    /// Reconstruct a ceremony for `epoch` from its journal after a restart. Rebuilds
    /// `Player.view` from the journaled dealings and logs, repopulates the recorded
    /// set including any equivocation pair, and re-emits one ack per peer dealing.
    ///
    /// `reconstruct_dealer` selects the shape described on [`Resumed`]. `preferred`
    /// names, per dealer, the body the rebuilt player should stand on when the journal
    /// holds more than one of that dealer's; an unknown or absent hash falls back to
    /// the first-recorded body, the one this node proposes and confirms.
    ///
    /// A truncated journal that dropped a publicly-acked dealing surfaces as `Err` so
    /// the caller can sit out the epoch gracefully.
    pub fn resume(
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
        records: Vec<JournalRecord>,
        reconstruct_dealer: bool,
        preferred: &BTreeMap<PeerPubkey, B256>,
    ) -> Result<Resumed, DkgError> {
        let me = me_key.public_key();
        let info = info_for(namespace, epoch, committee.clone())?;

        // Rebuild the recorded set through the same `insert_log` the live path uses, in
        // journal order, so a dealer's second log lands as the second half of its pair
        // rather than as a replacement of the first.
        let mut shell = Self {
            epoch,
            info: info.clone(),
            roster: committee,
            dealer: None,
            player: None,
            pending_pub: BTreeMap::new(),
            pending_priv: BTreeMap::new(),
            signed_logs: BTreeMap::new(),
            first_log: BTreeMap::new(),
            equivocations: BTreeMap::new(),
            own_pub_msg: None,
            unsent: BTreeMap::new(),
            emitted_acks: BTreeMap::new(),
        };
        // One `DealerLog` per dealer for `Player::resume`'s integrity check: the preferred
        // body where the caller names one and the journal holds it, else the first-recorded
        // log of each dealer.
        let mut log_map: BTreeMap<PeerPubkey, DealerLog<MinSig, PeerPubkey>> = BTreeMap::new();
        let mut file_log = |pk: PeerPubkey, log: DealerLog<MinSig, PeerPubkey>, hash: B256| {
            if preferred.get(&pk) == Some(&hash) {
                log_map.insert(pk, log);
            } else {
                log_map.entry(pk).or_insert(log);
            }
        };
        let mut own_seal = false;
        // Dealings to feed `Player::resume`, including our own self-dealing, which
        // rebuilds `view[me]` and regenerates our self-ack.
        let mut received: Vec<(PeerPubkey, DealerCommitment, DealerPrivMsg)> = Vec::new();
        // Acks our own dealer collected before the crash, replayed into the reconstructed
        // dealer so it does not re-reveal an already-acked player. Ignored player-only.
        let mut own_dealer_acks: Vec<(PeerPubkey, Ack)> = Vec::new();
        for record in records {
            match record {
                JournalRecord::ReceivedDealing(dealer, pub_msg, priv_msg) => {
                    received.push((dealer, *pub_msg, *priv_msg));
                }
                JournalRecord::OwnDealerAck(player, ack) => {
                    own_dealer_acks.push((player, *ack));
                }
                // Mark `own_seal` only after `check` succeeds and files our log: a tampered
                // `OwnSeal` frame that fails check must not mark the epoch sealed.
                JournalRecord::OwnSeal(_) => {
                    for (pk, log, signed) in checked_logs_in(record, &info, epoch) {
                        own_seal = true;
                        file_log(pk.clone(), log, log_hash(&signed));
                        let _ = shell.insert_log(pk, signed);
                    }
                }
                // An evidence record is taken whole or not at all. Its order is read — `first` is
                // the hash this node recorded first — and checked against the journal's own order,
                // which wins otherwise.
                JournalRecord::DealerEquivocation(..) => {
                    let halves = checked_logs_in(record, &info, epoch);
                    let claimed = match &halves[..] {
                        [(dealer, _, a), (_, _, b)] => Some((
                            dealer.clone(),
                            DealerEquivocation {
                                first: log_hash(a),
                                second: log_hash(b),
                            },
                        )),
                        _ => None,
                    };
                    for (pk, log, signed) in halves {
                        file_log(pk.clone(), log, log_hash(&signed));
                        let _ = shell.insert_log(pk, signed);
                    }
                    if let Some((dealer, claimed)) = claimed {
                        if shell.equivocation(&dealer) != Some(&claimed) {
                            tracing::warn!(
                                epoch,
                                %dealer,
                                claimed_first = %claimed.first,
                                claimed_second = %claimed.second,
                                "live DKG: journaled equivocation record names the pair in \
                                 another order than the journal recorded the bodies — the \
                                 journal's order stands"
                            );
                        }
                    }
                }
                JournalRecord::PeerLog(_) => {
                    for (pk, log, signed) in checked_logs_in(record, &info, epoch) {
                        file_log(pk.clone(), log, log_hash(&signed));
                        let _ = shell.insert_log(pk, signed);
                    }
                }
            }
        }

        // Rebuild `Player.view` and capture the regenerated per-dealer acks, including our
        // own self-ack under `me`.
        let (player, acks) =
            Player::resume::<N3f1>(info.clone(), me_key.clone(), &log_map, received)?;

        // Player ack-cache in both shapes: every dealer we validly acked, so a later
        // re-receipt re-emits the cached ack after a restart too.
        let emitted_acks: BTreeMap<PeerPubkey, Ack> =
            acks.iter().map(|(d, a)| (d.clone(), a.clone())).collect();

        // Re-emit an ack to every peer dealer whose dealing we replayed. The `me` self-ack
        // is not networked; it feeds our own reconstructed dealer below.
        let mut outgoing: Vec<Outgoing> = acks
            .iter()
            .filter(|(dealer, _)| **dealer != me)
            .map(|(dealer, ack)| Outgoing {
                target: Target::Direct(dealer.clone()),
                msg: DkgMsg {
                    ceremony_epoch: epoch,
                    body: DkgBody::Ack(ack.clone()),
                },
            })
            .collect();

        let (dealer, own_pub_msg, unsent) = if reconstruct_dealer {
            // Pre-seal restart: re-derive the identical dealer and resume both legs.
            match init_dealer(&me_key, epoch, &info) {
                Ok((mut dealer, pub_msg, mut unsent, _self_priv)) => {
                    // Replay the acks our dealer had already collected, pruning each from `unsent`.
                    for (player_pk, ack) in own_dealer_acks {
                        let _ = dealer.receive_player_ack(player_pk.clone(), ack);
                        unsent.remove(&player_pk);
                    }
                    // Feed our own self-ack so the restarted dealer does not reveal itself.
                    if let Some(self_ack) = acks.get(&me) {
                        let _ = dealer.receive_player_ack(me.clone(), self_ack.clone());
                        unsent.remove(&me);
                    }
                    (Some(dealer), Some(pub_msg), unsent)
                }
                Err(e) => {
                    // Degenerate under Model B. Degrade to player-only rather than sit the
                    // epoch out: the n-f survivors finalize and we recover our share.
                    tracing::warn!(
                        epoch,
                        ?e,
                        "live DKG: dealer reconstruct on resume failed — falling back to player-only"
                    );
                    (None, None, BTreeMap::new())
                }
            }
        } else {
            // At or after the seal deadline: player-only, never re-seal. If we sealed before
            // the crash, re-broadcast our journaled log verbatim.
            if own_seal {
                if let Some(signed) = shell
                    .first_log
                    .get(&me)
                    .and_then(|hash| shell.signed_logs.get(&(me.clone(), *hash)))
                {
                    outgoing.push(Outgoing {
                        target: Target::Broadcast,
                        msg: DkgMsg {
                            ceremony_epoch: epoch,
                            body: DkgBody::Reveal(Box::new(signed.clone())),
                        },
                    });
                }
            }
            (None, None, BTreeMap::new())
        };

        shell.dealer = dealer;
        shell.player = Some(player);
        shell.own_pub_msg = own_pub_msg;
        shell.unsent = unsent;
        shell.emitted_acks = emitted_acks;
        Ok(Resumed {
            ceremony: shell,
            outgoing,
        })
    }

    /// The content hash of the log this node recorded first for `dealer`, if any — the
    /// index the agreement proposes over and a `ShareConfirm` states. Stable for the
    /// ceremony's life, so what this node claimed stays what it claimed.
    pub fn signed_log_hash(&self, dealer: &PeerPubkey) -> Option<B256> {
        self.first_log.get(dealer).copied()
    }

    /// Build `Logs` restricted to exactly the pinned dealer-log set: for each
    /// `(idx, hash)` map `idx` to `committee[idx]` and include the recorded log held
    /// under exactly that `(dealer, hash)`. An absent body is reported by `idx`, so
    /// every honest node fetches the same pinned bytes and selects over the identical
    /// set.
    ///
    /// An `idx` with no position in `committee` is skipped rather than counted against
    /// `all_held`: nothing can satisfy it, because the resolver fetches over the roster
    /// and there is no dealer to name. The skip is a pure function of the pinned map and
    /// the committed committee, so every honest node scopes the identical set. If the
    /// mappable remainder cannot reach quorum the caller takes the below-quorum path.
    fn scoped_pinned_logs(
        &self,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> (Logs<MinSig, PeerPubkey, N3f1>, Vec<u8>) {
        let mut logs = Logs::<MinSig, PeerPubkey, N3f1>::new(self.info.clone());
        let mut missing = Vec::new();
        for (idx, hash) in pinned {
            let Some(pk) = committee.iter().nth(*idx as usize) else {
                continue; // unmappable idx — deterministic skip, see the docstring
            };
            match self.signed_logs.get(&(pk.clone(), *hash)) {
                Some(signed) => {
                    // Re-check (guaranteed to pass) to obtain the `DealerLog` for
                    // `Logs::record`. The checked signer must be the key the body is
                    // filed under; a mismatch is a corrupt map, reported as not held.
                    match signed.clone().check(&self.info) {
                        Some((cpk, log)) if cpk == *pk => {
                            logs.record(cpk, log);
                        }
                        Some((cpk, _)) => {
                            tracing::error!(
                                epoch = self.epoch,
                                filed_under = %pk,
                                signed_by = %cpk,
                                "live DKG: recorded-log map corrupt — a body filed under one \
                                 dealer is signed by another; treating the seat as not held"
                            );
                            missing.push(*idx);
                        }
                        None => missing.push(*idx),
                    }
                }
                None => missing.push(*idx), // not held under that exact hash
            }
        }
        (logs, missing)
    }

    /// Non-destructive finalize probe over the pinned set: `ready` when a selectable
    /// quorum exists within the pinned and held dealers, `all_held` when every mappable
    /// pinned body is held with a matching hash. Finalize only when `all_held && ready`.
    pub fn pinned_ready<R: CryptoRngCore>(
        &self,
        rng: &mut R,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> (bool, bool) {
        let (logs, missing) = self.scoped_pinned_logs(committee, pinned);
        let ready =
            observe::<MinSig, PeerPubkey, N3f1, ed25519::Batch>(rng, logs, &Sequential).is_ok();
        (ready, missing.is_empty())
    }

    /// The public half of the ceremony over exactly `pinned` — the ceremony-side half
    /// of [`crate::beacon::dkg_agree::PinnedLogs`].
    ///
    /// The arms are not interchangeable: only [`PinnedDerive::Unusable`] is a property
    /// of the pinned set, and it is reached only with every named body in hand. A body
    /// this node has not received is [`PinnedDerive::Missing`] naming the seats to
    /// fetch. The agreement turns `Unusable` into a nullified view for the whole
    /// network, and a node-local delivery gap must never cost that.
    ///
    /// An `idx` with no committee position answers `Unavailable` here, where
    /// [`scoped_pinned_logs`](Self::scoped_pinned_logs) skips it: on the ordering plane
    /// the pinned set is already agreed, whereas here it is a candidate and the skip
    /// could let `Unusable` be returned over a body this node never examined.
    pub fn derive_pinned<R: CryptoRngCore>(
        &self,
        rng: &mut R,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> PinnedDerive {
        if pinned.keys().any(|idx| *idx as usize >= committee.len()) {
            return PinnedDerive::Unavailable;
        }
        let (logs, missing) = self.scoped_pinned_logs(committee, pinned);
        if !missing.is_empty() {
            return PinnedDerive::Missing(missing);
        }
        match observe::<MinSig, PeerPubkey, N3f1, ed25519::Batch>(rng, logs, &Sequential) {
            Ok(output) => PinnedDerive::Derived(Box::new(output)),
            Err(_) => PinnedDerive::Unusable,
        }
    }

    /// Finalize `Player` over exactly the pinned set, which is what makes the derived
    /// `PK_E` a pure function of agreed data.
    ///
    /// Non-destructive to the ceremony: it consumes only the `Player`, leaving the
    /// recorded logs intact so the ceremony keeps serving peers.
    ///
    /// Caller contract: [`pinned_ready`](Self::pinned_ready) must have confirmed
    /// `ready` — a selectable quorum of the pinned and held dealers, without which
    /// `Player::finalize` returns `Err(DkgFailed)` — and `all_held`, so the output is
    /// derived over every mappable pinned body, not a locally held subset. On the
    /// dealing branch [`seal_dealings`](Self::seal_dealings) must have run first; a
    /// node resumed at or after the seal deadline is player-only and calls this
    /// directly. The player is consumed either way, so a second call answers
    /// [`FinalizeError::PlayerConsumed`].
    pub fn finalize_over_pinned<R: CryptoRngCore>(
        &mut self,
        rng: &mut R,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> Result<(CeremonyOutput, Share), FinalizeError> {
        let (logs, _missing) = self.scoped_pinned_logs(committee, pinned);
        let player = self.player.take().ok_or(FinalizeError::PlayerConsumed)?;
        Ok(player.finalize::<N3f1, ed25519::Batch>(rng, logs, &Sequential)?)
    }
}

/// Why [`DkgCeremony::finalize_over_pinned`] produced no share.
#[derive(Debug, thiserror::Error)]
pub enum FinalizeError {
    /// The ceremony's player was already consumed by an earlier finalize.
    #[error("the ceremony's player was already consumed")]
    PlayerConsumed,
    #[error(transparent)]
    Dkg(#[from] DkgError),
}

/// Locally recompute this node's share for a finalized epoch from its journal,
/// scoped to exactly the pinned `dealer → hash` set the artifact named.
///
/// Selecting over a superset would let `Logs::select` pick a different first quorum
/// and derive a different `PK_E`, so the scope matters. Player-only — never
/// re-deals. The caller must self-verify the returned share against the pinned
/// `Output` before adopting; a short, torn or tampered journal fails that check and
/// is never adopted.
pub fn recompute_scoped<R: CryptoRngCore>(
    rng: &mut R,
    namespace: &[u8],
    epoch: u64,
    committee: Set<PeerPubkey>,
    me_key: Ed25519PrivateKey,
    pinned: &BTreeMap<PeerPubkey, B256>,
    records: Vec<JournalRecord>,
) -> Result<(CeremonyOutput, Share), DkgError> {
    let info = info_for(namespace, epoch, committee)?;

    let mut log_map: BTreeMap<PeerPubkey, DealerLog<MinSig, PeerPubkey>> = BTreeMap::new();
    let mut received: Vec<(PeerPubkey, DealerCommitment, DealerPrivMsg)> = Vec::new();
    for record in records {
        match record {
            JournalRecord::ReceivedDealing(dealer, pub_msg, priv_msg) => {
                received.push((dealer, *pub_msg, *priv_msg));
            }
            // The player-state rebuild does not consume dealer-side acks.
            JournalRecord::OwnDealerAck(..) => {}
            // Keep only the bodies the pinned set names, by exact `(dealer, hash)`.
            JournalRecord::OwnSeal(_)
            | JournalRecord::PeerLog(_)
            | JournalRecord::DealerEquivocation(..) => {
                for signed in logs_in(record) {
                    if let Some((pk, log)) = signed.clone().check(&info) {
                        if pinned.get(&pk) == Some(&log_hash(&signed)) {
                            log_map.insert(pk, log);
                        }
                    }
                }
            }
        }
    }

    // Player-only resume (rebuild `view`); the acks it re-derives are discarded (we
    // are past the boundary, healing locally — nothing to re-broadcast).
    let (player, _acks) = Player::resume::<N3f1>(info.clone(), me_key, &log_map, received)?;
    let mut logs = Logs::<MinSig, PeerPubkey, N3f1>::new(info);
    for (pk, log) in log_map {
        logs.record(pk, log);
    }
    player.finalize::<N3f1, ed25519::Batch>(rng, logs, &Sequential)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::outcome::{group_public_key, validate_share_on_poly};
    use commonware_consensus::types::{Epoch, Round, View};
    use commonware_cryptography::bls12381::dkg::DealerLogSummary;
    use commonware_math::algebra::Random as _;
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial, verify_seed};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::collections::BTreeSet;

    /// A pinned `idx → hash` set mapped onto its committee, the shape `recompute_scoped`
    /// takes.
    fn by_dealer(
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> BTreeMap<PeerPubkey, B256> {
        pinned
            .iter()
            .map(|(idx, hash)| {
                (
                    committee.iter().nth(*idx as usize).expect("seat").clone(),
                    *hash,
                )
            })
            .collect()
    }

    /// The ceremony's own recorded dealer logs as a pinned set: every committee seat
    /// whose log it holds, under the hash it holds.
    fn pinned_over_recorded(cer: &DkgCeremony, committee: &Set<PeerPubkey>) -> BTreeMap<u8, B256> {
        committee
            .iter()
            .enumerate()
            .filter_map(|(idx, pk)| {
                cer.signed_log_hash(pk)
                    .map(|hash| (u8::try_from(idx).expect("committee fits a u8"), hash))
            })
            .collect()
    }

    /// Drive N ceremonies to completion through the event API, then assert every node
    /// agreed on `PK_E` and that the shares recover a verifiable seed.
    #[test]
    fn ceremonies_agree_and_feed_a_verifiable_seed_via_events() {
        let mut rng = StdRng::seed_from_u64(11);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }

        deliver_all(&mut ceremonies, &mut queue);

        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        deliver_all(&mut ceremonies, &mut queue);

        let mut outputs = Vec::new();
        let mut shares = BTreeMap::new();
        for (pk, mut cer) in ceremonies {
            let pinned = pinned_over_recorded(&cer, &committee);
            let (out, share) = cer
                .finalize_over_pinned(&mut rng, &committee, &pinned)
                .expect("finalize");
            shares.insert(pk, share);
            outputs.push(out);
        }
        assert_eq!(shares.len(), keys.len(), "every node derives a share");
        let pk0 = group_public_key(&outputs[0]);
        for o in &outputs[1..] {
            assert_eq!(group_public_key(o), pk0, "all nodes agree on PK_E");
        }

        let seed_ns = seed_namespace(ns);
        let round = Round::new(Epoch::new(0), View::new(100));
        let partials: Vec<_> = shares
            .values()
            .map(|s| sign_seed_partial(s, &seed_ns, round))
            .collect();
        let sig = recover_seed::<N3f1>(outputs[0].public(), &partials).expect("recover");
        assert!(
            verify_seed(pk0, &seed_ns, round, &sig),
            "seed from the event-driven DKG shares must verify against PK_E"
        );
    }

    /// Drive `n=5` ceremonies to the point where every node holds all five recorded
    /// dealer logs, and return the committee and ceremonies.
    fn run_to_all_sealed(seed: u64) -> (Set<PeerPubkey>, BTreeMap<PeerPubkey, DkgCeremony>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";
        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }
        deliver_all(&mut ceremonies, &mut queue);
        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        deliver_all(&mut ceremonies, &mut queue);
        (committee, ceremonies)
    }

    /// Build the pinned `idx → hash` map for a subset of committee positions, reading
    /// the content hash from `source`'s recorded logs.
    fn pinned_for(
        committee: &Set<PeerPubkey>,
        source: &DkgCeremony,
        idxs: &[u8],
    ) -> BTreeMap<u8, B256> {
        idxs.iter()
            .map(|&idx| {
                let pk = committee.iter().nth(idx as usize).expect("committee idx");
                (idx, source.signed_log_hash(pk).expect("recorded log"))
            })
            .collect()
    }

    /// Two different nodes that finalize over the identical pinned set derive the
    /// identical `PK_E`, for the full set and for a proper subset.
    /// The player is consumed by the first finalize; a second call over the same
    /// ceremony answers an error rather than stopping the actor.
    #[test]
    fn a_second_finalize_over_a_consumed_player_is_an_error_not_a_panic() {
        let mut rng = StdRng::seed_from_u64(0xF1);
        let (committee, mut ceremonies) = run_to_all_sealed(0xF1);
        let (first, cer) = ceremonies.iter_mut().next().expect("a node");
        let pinned = pinned_over_recorded(cer, &committee);
        cer.finalize_over_pinned(&mut rng, &committee, &pinned)
            .expect("the first finalize");
        assert!(
            matches!(
                cer.finalize_over_pinned(&mut rng, &committee, &pinned),
                Err(FinalizeError::PlayerConsumed)
            ),
            "node {first}: a second finalize must answer PlayerConsumed"
        );
    }

    #[test]
    fn finalize_over_pinned_is_pure_in_the_finalized_set() {
        let mut rng = StdRng::seed_from_u64(101);
        let (committee, mut ceremonies) = run_to_all_sealed(101);
        let nodes: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        // n=5, N3f1 means dealer quorum n-f = 4; `select` picks the lowest quorum valid
        // dealers, so `{0,1,2,3,4}` and `{0,1,2,3}` select the same four, while
        // `{1,2,3,4}` selects a different quorum.
        let lo = pinned_for(&committee, &ceremonies[&nodes[0]], &[0, 1, 2, 3]);
        let hi = pinned_for(&committee, &ceremonies[&nodes[0]], &[1, 2, 3, 4]);

        for pk in &nodes {
            let (ready, all_held) = ceremonies[pk].pinned_ready(&mut rng, &committee, &lo);
            assert!(ready && all_held, "quorum pinned set: ready + all held");
        }

        let (out_a, _) = ceremonies
            .get_mut(&nodes[0])
            .unwrap()
            .finalize_over_pinned(&mut rng, &committee, &lo)
            .expect("finalize a");
        let (out_b, _) = ceremonies
            .get_mut(&nodes[1])
            .unwrap()
            .finalize_over_pinned(&mut rng, &committee, &lo)
            .expect("finalize b");
        assert_eq!(
            group_public_key(&out_a),
            group_public_key(&out_b),
            "same pinned set ⇒ identical PK_E across nodes"
        );

        let (out_c, _) = ceremonies
            .get_mut(&nodes[2])
            .unwrap()
            .finalize_over_pinned(&mut rng, &committee, &hi)
            .expect("finalize c");
        let (out_d, _) = ceremonies
            .get_mut(&nodes[3])
            .unwrap()
            .finalize_over_pinned(&mut rng, &committee, &hi)
            .expect("finalize d");
        assert_eq!(
            group_public_key(&out_c),
            group_public_key(&out_d),
            "same (other) pinned set ⇒ identical PK_E across nodes"
        );
        assert_ne!(
            group_public_key(&out_a),
            group_public_key(&out_c),
            "a different selected dealer quorum yields a different key"
        );
    }

    /// A node whose local bodies are a subset of the pinned hashes reports
    /// `all_held=false`, so the actor waits instead of subset-finalizing.
    #[test]
    fn unmappable_pinned_idx_is_skipped_and_does_not_wedge() {
        let mut rng = StdRng::seed_from_u64(303);
        let (committee, mut ceremonies) = run_to_all_sealed(303);
        let nodes: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        let clean = pinned_for(&committee, &ceremonies[&nodes[0]], &[0, 1, 2, 3]);

        let mut bogus = clean.clone();
        bogus.insert(committee.len() as u8, B256::repeat_byte(0xAB));

        for pk in &nodes {
            let (ready, all_held) = ceremonies[pk].pinned_ready(&mut rng, &committee, &bogus);
            assert!(
                ready && all_held,
                "an unmappable idx must be skipped, not block the finalize gate"
            );
        }

        let (out_bogus, _) = ceremonies
            .get_mut(&nodes[0])
            .unwrap()
            .finalize_over_pinned(&mut rng, &committee, &bogus)
            .expect("finalize over the pinned set carrying an unmappable idx");
        let (out_clean, _) = ceremonies
            .get_mut(&nodes[1])
            .unwrap()
            .finalize_over_pinned(&mut rng, &committee, &clean)
            .expect("finalize over the clean pinned set");
        assert_eq!(
            group_public_key(&out_bogus),
            group_public_key(&out_clean),
            "skipping an unmappable idx must not move PK_E"
        );
    }

    #[test]
    fn pinned_ready_all_held_false_when_a_pinned_body_is_missing() {
        let mut rng = StdRng::seed_from_u64(202);
        let (committee, ceremonies) = run_to_all_sealed(202);
        let node = ceremonies.keys().next().unwrap().clone();
        let full = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3, 4]);
        let (_ready, all_held) = ceremonies[&node].pinned_ready(&mut rng, &committee, &full);
        assert!(all_held, "matching hashes ⇒ all held");
        let mut missing = full.clone();
        missing.insert(4, B256::repeat_byte(0xEE));
        let (_ready, all_held) = ceremonies[&node].pinned_ready(&mut rng, &committee, &missing);
        assert!(
            !all_held,
            "a pinned hash with no matching held body ⇒ WAIT (all_held=false)"
        );
    }

    /// A body this node does not hold is `Missing` naming the seats to fetch, never
    /// `Unusable`, which is the only arm the agreement may turn into a nullified view.
    #[test]
    fn derive_pinned_separates_a_missing_body_from_an_unusable_set() {
        let mut rng = StdRng::seed_from_u64(404);
        let (committee, ceremonies) = run_to_all_sealed(404);
        let node = ceremonies.keys().next().unwrap().clone();

        let quorum = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3]);
        let PinnedDerive::Derived(key) =
            ceremonies[&node].derive_pinned(&mut rng, &committee, &quorum)
        else {
            panic!("a held quorum must derive a key");
        };
        for other in ceremonies.keys() {
            let PinnedDerive::Derived(theirs) =
                ceremonies[other].derive_pinned(&mut rng, &committee, &quorum)
            else {
                panic!("every holder of the set derives from it");
            };
            assert_eq!(
                *theirs, *key,
                "the derived key is not a function of the set alone"
            );
        }

        let mut raced = quorum.clone();
        raced.insert(3, B256::repeat_byte(0xEE));
        assert!(
            matches!(
                ceremonies[&node].derive_pinned(&mut rng, &committee, &raced),
                PinnedDerive::Missing(ref idxs) if idxs == &vec![3u8]
            ),
            "a body this node does not hold must be Missing(seat), never Unusable"
        );

        let short = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2]);
        assert!(
            matches!(
                ceremonies[&node].derive_pinned(&mut rng, &committee, &short),
                PinnedDerive::Unusable
            ),
            "a fully-held set that yields no key is Unusable"
        );
    }

    /// An `idx` this node cannot map to a committee seat leaves its body unexamined, so
    /// `derive_pinned` answers `Unavailable` rather than a possibly-`Unusable` set.
    #[test]
    fn derive_pinned_is_unavailable_when_a_pinned_idx_has_no_committee_seat() {
        let mut rng = StdRng::seed_from_u64(405);
        let (committee, ceremonies) = run_to_all_sealed(405);
        let node = ceremonies.keys().next().unwrap().clone();

        let mut over = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3]);
        over.insert(committee.len() as u8, B256::repeat_byte(0xAB));
        assert!(
            matches!(
                ceremonies[&node].derive_pinned(&mut rng, &committee, &over),
                PinnedDerive::Unavailable
            ),
            "an unmappable idx is a disagreement about the roster — park, never vote"
        );

        let mapped = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3]);
        assert!(matches!(
            ceremonies[&node].derive_pinned(&mut rng, &committee, &mapped),
            PinnedDerive::Derived(_)
        ));
    }

    /// A pinned set below the reconstruction threshold is not `ready`; at quorum it is.
    #[test]
    fn pinned_ready_false_below_threshold_true_at_quorum() {
        let mut rng = StdRng::seed_from_u64(303);
        let (committee, ceremonies) = run_to_all_sealed(303);
        let node = ceremonies.keys().next().unwrap().clone();
        let three = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2]);
        let (ready, all_held) = ceremonies[&node].pinned_ready(&mut rng, &committee, &three);
        assert!(all_held, "the three bodies ARE held");
        assert!(
            !ready,
            "below quorum ⇒ no selectable quorum ⇒ not ready (defer)"
        );
        let four = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3]);
        let (ready, _) = ceremonies[&node].pinned_ready(&mut rng, &committee, &four);
        assert!(ready, "at quorum ⇒ a quorum is selectable ⇒ ready");
    }

    /// The seeded dealer is deterministic: two `start`s for the same `(key, epoch)`
    /// produce the byte-identical commitment, while a different epoch differs.
    #[test]
    fn seed_makes_dealer_start_idempotent() {
        let mut rng = StdRng::seed_from_u64(5);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";
        let me = keys[0].clone();

        let commit = |epoch: u64| {
            let (_c, step) =
                DkgCeremony::start(ns, epoch, committee.clone(), me.clone()).expect("start");
            step.outgoing
                .iter()
                .find_map(|o| match &o.msg.body {
                    DkgBody::Commitment(c) => Some(c.encode()),
                    _ => None,
                })
                .expect("commitment")
        };
        assert_eq!(
            commit(2),
            commit(2),
            "same (key, epoch) ⇒ byte-identical commitment (idempotent re-deal)"
        );
        assert_ne!(
            commit(2),
            commit(3),
            "a different epoch ⇒ a different commitment"
        );

        let a = me.sign(DEALER_SEED_NS, &2u64.to_be_bytes());
        let b = me.sign(DEALER_SEED_NS, &2u64.to_be_bytes());
        assert_eq!(
            a.as_ref(),
            b.as_ref(),
            "ed25519 signing (the seed source) is deterministic (RFC 8032)"
        );
    }

    /// `retransmit` starts non-empty, shrinks to empty as acks land, and returns nothing
    /// after `dealing_closed()`.
    #[test]
    fn retransmit_shrinks_as_acks_land_and_stops_after_seal() {
        let mut rng = StdRng::seed_from_u64(8);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }
        let d0 = keys[0].public_key();
        assert_eq!(ceremonies[&d0].retransmit().len(), 6);

        deliver_all(&mut ceremonies, &mut queue);
        assert!(
            ceremonies[&d0].retransmit().is_empty(),
            "every player acked ⇒ unsent drained ⇒ nothing to retransmit"
        );

        let _ = ceremonies.get_mut(&d0).expect("d0").seal_dealings();
        assert!(ceremonies[&d0].dealing_closed());
        assert!(
            ceremonies[&d0].retransmit().is_empty(),
            "a sealed ceremony never retransmits"
        );
    }

    /// Both delivery legs heal via retransmit and ack re-emit: with a dealing and an ack
    /// dropped, the dealer's sealed log reveals neither player.
    #[test]
    fn retransmit_heals_dropped_dealing_and_ack_no_reveals() {
        let mut rng = StdRng::seed_from_u64(3);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";
        let d0 = keys[0].public_key();
        let p1 = keys[1].public_key();
        let p2 = keys[2].public_key();

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }

        let d0c = d0.clone();
        let p1c = p1.clone();
        let p2c = p2.clone();
        let drop = move |from: &PeerPubkey, to: &PeerPubkey, body: &DkgBody| -> bool {
            let dealer_leg = *from == d0c
                && *to == p1c
                && matches!(body, DkgBody::Commitment(_) | DkgBody::Share(_));
            let ack_leg = *from == p2c && *to == d0c && matches!(body, DkgBody::Ack(_));
            dealer_leg || ack_leg
        };
        deliver_dropping(&mut ceremonies, &mut queue, &drop);
        assert!(
            ceremonies[&d0].unsent.contains_key(&p1),
            "player-1 un-acked (its dealing was dropped)"
        );
        assert!(
            ceremonies[&d0].unsent.contains_key(&p2),
            "player-2 un-acked (its ack was dropped)"
        );

        for _ in 0..2 {
            let retx: Vec<(PeerPubkey, Outgoing)> = ceremonies
                .iter()
                .flat_map(|(from, c)| c.retransmit().into_iter().map(|o| (from.clone(), o)))
                .collect();
            queue.extend(retx);
            deliver_all(&mut ceremonies, &mut queue);
        }
        assert!(
            ceremonies[&d0].unsent.is_empty(),
            "both legs healed: dealer-0's unsent drained"
        );

        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        let info = info_for(ns, 0, committee.clone()).expect("info");
        let own_hash = ceremonies[&d0]
            .signed_log_hash(&d0)
            .expect("dealer-0 sealed");
        let own = ceremonies[&d0]
            .signed_log(&(d0.clone(), own_hash))
            .expect("dealer-0 sealed")
            .clone();
        let (dpk, log) = own.check(&info).expect("checks");
        assert_eq!(dpk, d0);
        match log.summary() {
            DealerLogSummary::Ok { reveals, .. } => assert!(
                !reveals.iter().any(|p| *p == p1 || *p == p2),
                "neither the dropped-dealing nor the dropped-ack player is revealed"
            ),
            DealerLogSummary::TooManyReveals => panic!("dealer-0 must not be TooManyReveals"),
        }

        deliver_all(&mut ceremonies, &mut queue);
        let outputs: Vec<CeremonyOutput> = ceremonies
            .into_values()
            .map(|mut c| {
                let pinned = pinned_over_recorded(&c, &committee);
                c.finalize_over_pinned(&mut rng, &committee, &pinned)
                    .expect("finalize")
                    .0
            })
            .collect();
        let pk0 = group_public_key(&outputs[0]);
        for o in &outputs[1..] {
            assert_eq!(
                group_public_key(o),
                pk0,
                "all nodes agree on PK_E after a lossy-then-healed delivery"
            );
        }
    }

    /// `deliver_all` but drops any `(from, to, body)` for which `drop` returns true,
    /// modelling per-leg packet loss.
    fn deliver_dropping(
        ceremonies: &mut BTreeMap<PeerPubkey, DkgCeremony>,
        queue: &mut Vec<(PeerPubkey, Outgoing)>,
        drop: &impl Fn(&PeerPubkey, &PeerPubkey, &DkgBody) -> bool,
    ) {
        while let Some((from, o)) = queue.pop() {
            match o.target {
                Target::Broadcast => {
                    let recipients: Vec<PeerPubkey> = ceremonies
                        .keys()
                        .filter(|pk| **pk != from)
                        .cloned()
                        .collect();
                    for to in recipients {
                        if drop(&from, &to, &o.msg.body) {
                            continue;
                        }
                        let more = ceremonies
                            .get_mut(&to)
                            .expect("ceremony")
                            .handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
                Target::Direct(to) => {
                    if drop(&from, &to, &o.msg.body) {
                        continue;
                    }
                    if let Some(cer) = ceremonies.get_mut(&to) {
                        let more = cer.handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            }
        }
    }

    /// Drive a four-party ceremony to the point where node 0 has acked every dealer and
    /// sealed, capturing node 0's journal records.
    fn run_to_node0_sealed(seed: u64) -> (Set<PeerPubkey>, Ed25519PrivateKey, Vec<JournalRecord>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        let pk0 = keys[0].public_key();
        let mut journal0: Vec<JournalRecord> = Vec::new();

        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            if from == pk0 {
                journal0.extend(step.journal);
            }
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }
        deliver_capturing(&mut ceremonies, &mut queue, &pk0, &mut journal0);

        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            if pk == pk0 {
                journal0.extend(step.journal);
            }
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        deliver_capturing(&mut ceremonies, &mut queue, &pk0, &mut journal0);

        (committee, keys[0].clone(), journal0)
    }

    /// `deliver_all` but also appends node-`me`'s journal records as it handles each
    /// message.
    fn deliver_capturing(
        ceremonies: &mut BTreeMap<PeerPubkey, DkgCeremony>,
        queue: &mut Vec<(PeerPubkey, Outgoing)>,
        me: &PeerPubkey,
        journal_me: &mut Vec<JournalRecord>,
    ) {
        while let Some((from, o)) = queue.pop() {
            match o.target {
                Target::Broadcast => {
                    let recipients: Vec<PeerPubkey> = ceremonies
                        .keys()
                        .filter(|pk| **pk != from)
                        .cloned()
                        .collect();
                    for to in recipients {
                        let step = ceremonies
                            .get_mut(&to)
                            .expect("ceremony")
                            .handle(from.clone(), o.msg.body.clone());
                        if to == *me {
                            journal_me.extend(step.journal);
                        }
                        queue.extend(step.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
                Target::Direct(to) => {
                    if let Some(cer) = ceremonies.get_mut(&to) {
                        let step = cer.handle(from.clone(), o.msg.body.clone());
                        if to == *me {
                            journal_me.extend(step.journal);
                        }
                        queue.extend(step.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            }
        }
    }

    /// A node that journaled its dealings, own seal and peer logs rebuilds the identical
    /// `(PK_E, share)` via `Player::resume`.
    #[test]
    fn resume_after_own_seal_reproduces_identical_share() {
        let (committee, key0, journal0) = run_to_node0_sealed(31);
        let roster = committee.clone();

        let (committee2, key0b, journal0b) = run_to_node0_sealed(31);
        let mut rng_live = StdRng::seed_from_u64(31);
        let me0 = key0.public_key();
        let mut live = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee2,
            key0b,
            journal0b,
            false,
            &BTreeMap::new(),
        )
        .expect("live resume");
        assert!(
            live.ceremony.own_log_recorded(&me0),
            "own-seal record ⇒ our own log is recorded (no re-deal)"
        );
        assert!(
            live.ceremony.dealing_closed(),
            "player-only restore retires the dealer role"
        );
        let pinned_live = pinned_over_recorded(&live.ceremony, &roster);
        let (out_live, share_live) = live
            .ceremony
            .finalize_over_pinned(&mut rng_live, &roster, &pinned_live)
            .expect("finalize live");

        let mut rng_res = StdRng::seed_from_u64(99);
        let mut resumed = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee,
            key0,
            journal0,
            false,
            &BTreeMap::new(),
        )
        .expect("resume");
        assert!(resumed.ceremony.own_log_recorded(&me0));
        let pinned_res = pinned_over_recorded(&resumed.ceremony, &roster);
        let (out_res, share_res) = resumed
            .ceremony
            .finalize_over_pinned(&mut rng_res, &roster, &pinned_res)
            .expect("finalize resumed");

        assert_eq!(
            group_public_key(&out_res),
            group_public_key(&out_live),
            "resume derives the identical PK_E"
        );
        assert_eq!(
            share_res.encode().as_ref(),
            share_live.encode().as_ref(),
            "resume derives the identical secret share"
        );
    }

    /// A truncated journal that dropped a publicly-acked dealing surfaces
    /// `MissingPlayerDealing` on resume.
    #[test]
    fn resume_with_missing_acked_dealing_reports_missing_player_dealing() {
        let (committee, key0, mut journal0) = run_to_node0_sealed(42);
        let me0 = key0.public_key();
        let idx = journal0
            .iter()
            .position(|r| matches!(r, JournalRecord::ReceivedDealing(d, _, _) if *d != me0))
            .expect("a peer dealing");
        journal0.remove(idx);

        match DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee,
            key0,
            journal0,
            false,
            &BTreeMap::new(),
        ) {
            Err(DkgError::MissingPlayerDealing) => {}
            Err(other) => panic!("expected MissingPlayerDealing, got {other:?}"),
            Ok(_) => panic!("a missing acked dealing must fail resume"),
        }
    }

    /// A pre-deadline resume re-derives the identical seeded dealer and keeps
    /// distributing: the reconstructed dealer's `unsent` covers every peer and its
    /// commitment is byte-identical to the original `start`.
    #[test]
    fn pre_deadline_resume_reconstructs_identical_dealer() {
        let (committee, key0, journal0) = run_to_node0_sealed(13);
        let received_only: Vec<JournalRecord> = journal0
            .into_iter()
            .filter(|r| matches!(r, JournalRecord::ReceivedDealing(..)))
            .collect();

        let (_orig, orig_step) =
            DkgCeremony::start(b"FLUENT_DPOS_V1_test", 0, committee.clone(), key0.clone())
                .expect("start");
        let orig_commitment = orig_step
            .outgoing
            .iter()
            .find_map(|o| match &o.msg.body {
                DkgBody::Commitment(c) => Some(c.encode()),
                _ => None,
            })
            .expect("original commitment");

        let resumed = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee,
            key0,
            received_only,
            true,
            &BTreeMap::new(),
        )
        .expect("pre-deadline resume");
        assert!(
            !resumed.ceremony.dealing_closed(),
            "a pre-deadline resume KEEPS a live dealer (reconstructed), not player-only"
        );
        let retx = resumed.ceremony.retransmit();
        assert!(
            !retx.is_empty(),
            "the reconstructed dealer keeps distributing to its un-acked players"
        );
        let retx_commitment = retx
            .iter()
            .find_map(|o| match &o.msg.body {
                DkgBody::Commitment(c) => Some(c.encode()),
                _ => None,
            })
            .expect("retransmit re-sends the commitment");
        assert_eq!(
            retx_commitment, orig_commitment,
            "the reconstructed dealer re-derives the byte-identical commitment (seeded idempotency)"
        );
    }

    /// A reconstructed dealer that received every ack plus its own self-ack seals a log
    /// with zero reveals — it does not reveal itself.
    #[test]
    fn reconstructed_dealer_does_not_reveal_itself() {
        let (committee, key0, journal0) = run_to_node0_sealed(53);
        let me0 = key0.public_key();
        let pre_seal: Vec<JournalRecord> = journal0
            .into_iter()
            .filter(|r| !matches!(r, JournalRecord::OwnSeal(_)))
            .collect();

        let mut resumed = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee.clone(),
            key0,
            pre_seal,
            true,
            &BTreeMap::new(),
        )
        .expect("pre-deadline resume");
        assert!(
            !resumed.ceremony.dealing_closed(),
            "reconstructed a live dealer to seal at the deadline"
        );

        let own_log = resumed
            .ceremony
            .seal_dealings()
            .outgoing
            .into_iter()
            .find_map(|o| match o.msg.body {
                DkgBody::Reveal(s) => Some(*s),
                _ => None,
            })
            .expect("the reconstructed dealer seals + broadcasts its own log");
        let info = info_for(b"FLUENT_DPOS_V1_test", 0, committee).expect("info");
        let (pk, log) = own_log.check(&info).expect("own sealed log checks");
        assert_eq!(pk, me0);
        match log.summary() {
            DealerLogSummary::Ok { reveals, .. } => assert!(
                reveals.iter().next().is_none(),
                "a reconstructed dealer that received every ack reveals NO player \
                 (0 reveals — it does not self-reveal)"
            ),
            DealerLogSummary::TooManyReveals => {
                panic!("the reconstructed dealer log must not be TooManyReveals")
            }
        }
    }

    /// A resume re-emits one ack per peer dealing it replayed, and carries the
    /// re-broadcast own log on the own-seal path.
    #[test]
    fn resume_reemits_peer_acks() {
        let (committee, key0, journal0) = run_to_node0_sealed(17);
        let me0 = key0.public_key();
        let peer_dealers: BTreeSet<PeerPubkey> = journal0
            .iter()
            .filter_map(|r| match r {
                JournalRecord::ReceivedDealing(d, _, _) if *d != me0 => Some(d.clone()),
                _ => None,
            })
            .collect();
        assert!(!peer_dealers.is_empty(), "node-0 acked at least one peer");

        let resumed = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee,
            key0,
            journal0,
            false,
            &BTreeMap::new(),
        )
        .expect("resume");
        let acked: BTreeSet<PeerPubkey> = resumed
            .outgoing
            .iter()
            .filter_map(|o| match (&o.target, &o.msg.body) {
                (Target::Direct(pk), DkgBody::Ack(_)) => Some(pk.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            acked, peer_dealers,
            "resume re-emits exactly one Ack per replayed peer dealing"
        );
    }

    /// `recompute_scoped` is a deterministic function of the pinned set, independent of
    /// journal acquisition order or held supersets, so two honest nodes recompute the
    /// same share for one seat.
    #[test]
    fn recompute_scoped_is_deterministic_over_pinned_dealers() {
        let (committee, key0, journal_canon) = run_to_node0_sealed(71);
        let (_c2, _k2, journal_recompute) = run_to_node0_sealed(71);
        let (_c3, _k3, journal_reversed) = run_to_node0_sealed(71);
        let me0 = key0.public_key();

        let mut rng = StdRng::seed_from_u64(1);
        let mut canon = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee.clone(),
            key0.clone(),
            journal_canon,
            false,
            &BTreeMap::new(),
        )
        .expect("resume");
        let pinned_canon = pinned_over_recorded(&canon.ceremony, &committee);
        let (outcome, canonical_share) = canon
            .ceremony
            .finalize_over_pinned(&mut rng, &committee, &pinned_canon)
            .expect("finalize");

        // A different rng, since recompute is deterministic.
        let mut rng2 = StdRng::seed_from_u64(999); // a different rng — recompute is deterministic
        let scope = by_dealer(&committee, &pinned_canon);
        let (recomputed_out, recomputed_share) = recompute_scoped(
            &mut rng2,
            b"FLUENT_DPOS_V1_test",
            0,
            committee.clone(),
            key0.clone(),
            &scope,
            journal_recompute,
        )
        .expect("recompute");
        assert_eq!(
            recomputed_share.encode().as_ref(),
            canonical_share.encode().as_ref(),
            "recompute scoped to pinned dealers() reproduces the byte-identical share"
        );
        assert_eq!(
            group_public_key(&recomputed_out),
            group_public_key(&outcome),
            "recompute derives the identical PK_E"
        );
        assert!(
            validate_share_on_poly(&outcome, &committee, &key0.public_key(), &recomputed_share),
            "the recomputed share self-verifies against the pinned Output"
        );

        let mut rev = journal_reversed;
        rev.reverse();
        let mut rng3 = StdRng::seed_from_u64(7);
        let (_out_r, share_rev) = recompute_scoped(
            &mut rng3,
            b"FLUENT_DPOS_V1_test",
            0,
            committee,
            key0,
            &scope,
            rev,
        )
        .expect("recompute reversed");
        assert_eq!(
            share_rev.encode().as_ref(),
            canonical_share.encode().as_ref(),
            "recompute is independent of journal acquisition order (scoped to the pinned set)"
        );
        let _ = me0;
    }

    /// A second valid log of `dealer` over the same `Info`, dealt from an independent
    /// polynomial with no acks.
    fn second_log_of(
        dealer: &Ed25519PrivateKey,
        committee: &Set<PeerPubkey>,
        epoch: u64,
        seed: u64,
    ) -> DealerReveal {
        let info = info_for(b"FLUENT_DPOS_V1_test", epoch, committee.clone()).expect("info");
        let (d, _, _) = Dealer::<MinSig, Ed25519PrivateKey>::start::<N3f1>(
            StdRng::seed_from_u64(seed),
            info,
            dealer.clone(),
            None,
        )
        .expect("second dealer");
        d.finalize::<N3f1>()
    }

    /// Two valid logs of one dealer: both held under their own `(dealer, hash)`, the
    /// dealer proven equivocating once, a third gossip log dropped while the same log
    /// fetched by exact hash is recorded, and the pinned set deciding which body counts.
    #[test]
    fn a_dealers_second_valid_log_is_evidence_and_a_ban_but_never_a_replacement() {
        let mut rng = StdRng::seed_from_u64(77);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";
        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }
        deliver_all(&mut ceremonies, &mut queue);
        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        deliver_all(&mut ceremonies, &mut queue);

        let (me0, d1) = (keys[0].public_key(), keys[1].public_key());
        let h1 = ceremonies[&me0]
            .signed_log_hash(&d1)
            .expect("node 1 sealed");
        let log1 = ceremonies[&me0]
            .signed_log(&(d1.clone(), h1))
            .expect("held")
            .clone();
        let log2 = second_log_of(&keys[1], &committee, 0, 0x5EC0);
        let h2 = log_hash(&log2);
        assert_ne!(h1, h2, "the second log is a different body");
        let log3 = second_log_of(&keys[1], &committee, 0, 0x5EC1);
        let h3 = log_hash(&log3);
        assert!(h3 != h1 && h3 != h2);

        let node0 = ceremonies.get_mut(&me0).expect("node 0");
        assert_eq!(node0.recorded_log_count(), 4);
        let step = node0.handle(d1.clone(), DkgBody::Reveal(Box::new(log2.clone())));
        assert_eq!(step.recorded_log, Some((d1.clone(), h2)));
        assert_eq!(step.equivocation, Some(d1.clone()));
        assert!(
            matches!(&step.journal[..], [JournalRecord::DealerEquivocation(a, b)]
                if log_hash(a) == h1 && log_hash(b) == h2),
            "the second log is journaled INSIDE the evidence record, in arrival order"
        );
        assert!(node0.holds(&(d1.clone(), h1)) && node0.holds(&(d1.clone(), h2)));
        assert_eq!(node0.recorded_log_count(), 5, "held beside, not instead");
        assert_eq!(
            node0.equivocation(&d1),
            Some(&DealerEquivocation {
                first: h1,
                second: h2
            })
        );
        assert_eq!(
            node0.signed_log_hash(&d1),
            Some(h1),
            "what this node proposes/confirms for the seat stays the FIRST hash"
        );
        let dup = node0.handle(d1.clone(), DkgBody::Reveal(Box::new(log1.clone())));
        assert!(dup.journal.is_empty() && dup.equivocation.is_none());

        let banned = node0.handle(d1.clone(), DkgBody::Reveal(Box::new(log3.clone())));
        assert!(banned.journal.is_empty() && banned.recorded_log.is_none());
        assert!(
            !node0.holds(&(d1.clone(), h3)),
            "gossip from a banned dealer records nothing"
        );
        let (wrong, step) = node0.ingest_signed_log(&(d1.clone(), h1), log3.clone());
        assert!(
            !wrong && step.journal.is_empty(),
            "a body other than the one fetched"
        );
        let (ok, step) = node0.ingest_signed_log(&(d1.clone(), h3), log3.clone());
        assert!(ok);
        assert!(
            matches!(&step.journal[..], [JournalRecord::PeerLog(l)] if log_hash(l) == h3),
            "a further log of a proven equivocator is a plain PeerLog — the evidence is the first pair"
        );
        assert!(step.equivocation.is_none(), "the pair is created once");
        assert!(node0.holds(&(d1.clone(), h3)));
        assert_eq!(
            node0.equivocation(&d1),
            Some(&DealerEquivocation {
                first: h1,
                second: h2
            }),
            "the evidence stays the first pair"
        );

        let pinned_log1 = pinned_over_recorded(&ceremonies[&keys[2].public_key()], &committee);
        let seat1 = committee.iter().position(|pk| *pk == d1).expect("seat") as u8;
        assert_eq!(pinned_log1.get(&seat1), Some(&h1));
        let PinnedDerive::Derived(key_from_0) =
            ceremonies[&me0].derive_pinned(&mut rng, &committee, &pinned_log1)
        else {
            panic!("node 0 holds every pinned body");
        };
        let PinnedDerive::Derived(key_from_2) =
            ceremonies[&keys[2].public_key()].derive_pinned(&mut rng, &committee, &pinned_log1)
        else {
            panic!("node 2 holds every pinned body");
        };
        assert_eq!(group_public_key(&key_from_0), group_public_key(&key_from_2));
        let mut pinned_log2 = pinned_log1.clone();
        pinned_log2.insert(seat1, h2);
        assert!(matches!(
            ceremonies[&keys[2].public_key()].derive_pinned(&mut rng, &committee, &pinned_log2),
            PinnedDerive::Missing(ref idxs) if idxs == &vec![seat1]
        ));
        let (_ready, all_held) =
            ceremonies[&keys[2].public_key()].pinned_ready(&mut rng, &committee, &pinned_log2);
        assert!(!all_held);
        let (out0, _share0) = ceremonies
            .get_mut(&me0)
            .expect("node 0")
            .finalize_over_pinned(&mut rng, &committee, &pinned_log1)
            .expect("finalize");
        assert_eq!(group_public_key(&out0), group_public_key(&key_from_2));
    }

    /// A replayed journal rebuilds exactly the recorded `(dealer, hash)` set the live
    /// ceremony held, both halves of an equivocation pair included.
    #[test]
    fn a_replayed_journal_rebuilds_the_recorded_set_and_the_equivocation_pair() {
        let (committee, key0, mut journal0) = run_to_node0_sealed(83);
        let keys: Vec<Ed25519PrivateKey> = {
            let mut rng = StdRng::seed_from_u64(83);
            (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect()
        };
        assert_eq!(keys[0].public_key(), key0.public_key());
        let d1 = keys[1].public_key();
        let ns = b"FLUENT_DPOS_V1_test";

        let mut live = DkgCeremony::resume(
            ns,
            0,
            committee.clone(),
            key0.clone(),
            std::mem::take(&mut journal0),
            false,
            &BTreeMap::new(),
        )
        .expect("live")
        .ceremony;
        let log2 = second_log_of(&keys[1], &committee, 0, 0x5EC2);
        let h1 = live.signed_log_hash(&d1).expect("node 1's sealed log");
        let h2 = log_hash(&log2);
        let step = live.handle(d1.clone(), DkgBody::Reveal(Box::new(log2)));
        assert_eq!(step.equivocation, Some(d1.clone()));
        let (_c, _k, mut journal) = run_to_node0_sealed(83);
        journal.extend(step.journal);
        let before: Vec<LogId> = live.signed_logs.keys().cloned().collect();
        assert!(before.contains(&(d1.clone(), h1)) && before.contains(&(d1.clone(), h2)));
        assert_eq!(before.len(), 5);

        let resumed = DkgCeremony::resume(
            ns,
            0,
            committee.clone(),
            key0,
            journal,
            false,
            &BTreeMap::new(),
        )
        .expect("resume")
        .ceremony;
        let after: Vec<LogId> = resumed.signed_logs.keys().cloned().collect();
        assert_eq!(
            after, before,
            "the recorded (dealer, hash) set survives the restart"
        );
        assert_eq!(
            resumed.first_log, live.first_log,
            "and so does which hash is this node's first for each dealer"
        );
        assert_eq!(
            resumed.equivocation(&d1),
            Some(&DealerEquivocation {
                first: h1,
                second: h2
            }),
            "the evidence pair survives the restart"
        );
        assert!(resumed.own_log_recorded(&keys[0].public_key()));
        let mut resumed = resumed;
        let log3 = second_log_of(&keys[1], &committee, 0, 0x5EC3);
        let h3 = log_hash(&log3);
        let dropped = resumed.handle(d1.clone(), DkgBody::Reveal(Box::new(log3)));
        assert!(dropped.recorded_log.is_none() && !resumed.holds(&(d1, h3)));
    }

    /// One validity rule for a journaled pair on both readers: a pair whose halves name
    /// two dealers or are one body evidences nothing, while a genuine pair yields both
    /// bodies. A body the journal also holds on its own is still taken from there.
    #[test]
    fn the_serve_map_takes_a_journaled_pair_under_the_rule_the_resume_applies() {
        let (committee, key0, journal0) = run_to_node0_sealed(89);
        let keys: Vec<Ed25519PrivateKey> = {
            let mut rng = StdRng::seed_from_u64(89);
            (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect()
        };
        assert_eq!(keys[0].public_key(), key0.public_key());
        let ns = b"FLUENT_DPOS_V1_test";
        let info = info_for(ns, 0, committee.clone()).expect("info");
        let (d1, d2) = (keys[1].public_key(), keys[2].public_key());
        let sealed_log_of = |pk: &PeerPubkey| -> DealerReveal {
            journal0
                .iter()
                .find_map(|r| match r {
                    JournalRecord::PeerLog(l)
                        if l.clone().check(&info).is_some_and(|(p, _)| p == *pk) =>
                    {
                        Some((**l).clone())
                    }
                    _ => None,
                })
                .expect("sealed log journaled")
        };
        let (log1, log2) = (sealed_log_of(&d1), sealed_log_of(&d2));
        let log1b = second_log_of(&keys[1], &committee, 0, 0x5EC4);
        let (h1, h1b) = (log_hash(&log1), log_hash(&log1b));
        assert_ne!(h1, h1b);
        let base = || -> Vec<JournalRecord> {
            let (_, _, journal) = run_to_node0_sealed(89);
            let n = journal.len();
            let base: Vec<JournalRecord> = journal
                .into_iter()
                .filter(|r| match r {
                    JournalRecord::PeerLog(l) => !l
                        .clone()
                        .check(&info)
                        .is_some_and(|(p, _)| p == d1 || p == d2),
                    _ => true,
                })
                .collect();
            assert_eq!(base.len(), n - 2);
            base
        };
        let of_the_two = |ids: Vec<LogId>| -> BTreeSet<LogId> {
            ids.into_iter()
                .filter(|(pk, _)| *pk == d1 || *pk == d2)
                .collect()
        };
        let served = |shape: Vec<JournalRecord>| -> BTreeSet<LogId> {
            let mut records = base();
            records.extend(shape);
            of_the_two(
                checked_serve_map(ns, 0, committee.clone(), records)
                    .expect("serve map")
                    .into_keys()
                    .collect(),
            )
        };
        let held = |shape: Vec<JournalRecord>| -> BTreeSet<LogId> {
            let mut records = base();
            records.extend(shape);
            of_the_two(
                DkgCeremony::resume(
                    ns,
                    0,
                    committee.clone(),
                    key0.clone(),
                    records,
                    false,
                    &BTreeMap::new(),
                )
                .expect("resume")
                .ceremony
                .signed_logs
                .into_keys()
                .collect(),
            )
        };
        let pair = |a: &DealerReveal, b: &DealerReveal| {
            JournalRecord::DealerEquivocation(Box::new(a.clone()), Box::new(b.clone()))
        };

        let both = BTreeSet::from([(d1.clone(), h1), (d1.clone(), h1b)]);
        assert_eq!(
            served(vec![pair(&log1, &log1b)]),
            both,
            "a genuine pair is served whole"
        );
        assert_eq!(held(vec![pair(&log1, &log1b)]), both, "and replayed whole");
        assert!(
            served(vec![pair(&log1, &log2)]).is_empty(),
            "a pair naming two dealers is not served, half or whole"
        );
        assert!(
            held(vec![pair(&log1, &log2)]).is_empty(),
            "and not replayed"
        );
        assert!(
            served(vec![pair(&log1, &log1)]).is_empty(),
            "a one-body pair is not served"
        );
        assert!(
            held(vec![pair(&log1, &log1)]).is_empty(),
            "and not replayed"
        );
        let with_own = || {
            vec![
                pair(&log1, &log2),
                JournalRecord::PeerLog(Box::new(log1.clone())),
            ]
        };
        let own = BTreeSet::from([(d1.clone(), h1)]);
        assert_eq!(served(with_own()), own);
        assert_eq!(held(with_own()), own);
    }

    fn deliver_all(
        ceremonies: &mut BTreeMap<PeerPubkey, DkgCeremony>,
        queue: &mut Vec<(PeerPubkey, Outgoing)>,
    ) {
        while let Some((from, o)) = queue.pop() {
            match o.target {
                Target::Broadcast => {
                    let recipients: Vec<PeerPubkey> = ceremonies
                        .keys()
                        .filter(|pk| **pk != from)
                        .cloned()
                        .collect();
                    for to in recipients {
                        let more = ceremonies
                            .get_mut(&to)
                            .expect("ceremony")
                            .handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
                Target::Direct(to) => {
                    if let Some(cer) = ceremonies.get_mut(&to) {
                        let more = cer.handle(from.clone(), o.msg.body.clone());
                        queue.extend(more.outgoing.into_iter().map(|m| (to.clone(), m)));
                    }
                }
            }
        }
    }
}
