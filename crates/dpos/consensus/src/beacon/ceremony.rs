//! Event-driven DKG ceremony state machine — `run_local_dkg`'s logic decomposed
//! into message handlers so the networked actor can drive one node's dealer +
//! player roles over `BEACON_CHANNEL` (Model B: `committee[E]` deals to itself,
//! dealers == players).
//!
//! Lifecycle (two phases, separated by the height-pinned collection deadline):
//! 1. DEALING — [`DkgCeremony::start`] emits this node's commitment (broadcast) +
//!    one private share per other player (point-to-point); incoming
//!    `Commitment`+`Share` from each dealer are buffered until both are present,
//!    then acked; incoming `Ack`s prune this node's own-dealer `unsent` set + feed
//!    its dealer. Delivery is RELIABLE on BOTH legs (§8.11.1): the dealer re-sends
//!    each un-acked dealing every pre-seal tick ([`DkgCeremony::retransmit`]), and a
//!    player re-emits its CACHED ack on any re-receipt of a dealing it already acked
//!    ([`DkgCeremony::try_ack`]'s re-receipt arm), so a member reachable ≥ 1
//!    round-trip before seal is never reveal-forced by a single dropped packet.
//! 2. At the deadline [`DkgCeremony::seal_dealings`] finalizes this node's dealer
//!    into a signed log (broadcast as `Reveal`, recorded locally); incoming
//!    `Reveal`s are recorded. [`DkgCeremony::finalize`] then derives the agreed
//!    [`Output`] (`PK_E`) + this node's secret [`Share`] over the collected logs.
//!
//! `Player::finalize` is intentionally DEFERRED to [`finalize`] (the boundary),
//! never over a locally-selected `Q` mid-flight.

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
use std::collections::{BTreeMap, BTreeSet};

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

/// What one ceremony step produced: outgoing messages to broadcast/send AND the
/// durable [`JournalRecord`]s the actor appends so a restart can `resume` (§8.11.1).
#[derive(Default)]
pub struct Step {
    pub outgoing: Vec<Outgoing>,
    pub journal: Vec<JournalRecord>,
}

impl Step {
    fn out(outgoing: Vec<Outgoing>) -> Self {
        Self {
            outgoing,
            journal: Vec::new(),
        }
    }

    /// `true` iff this step recorded a NEW dealer log (its own seal or a peer's) into the
    /// finalizable set — the ONLY journal record that can change finalizability. An
    /// ack-only / `ReceivedDealing`-only step does not, so the actor can skip the
    /// `drive_finalization` (= `observe` batch-BLS over a `Logs` clone) it would otherwise
    /// run after EVERY inbound body (review [806]).
    pub fn recorded_a_log(&self) -> bool {
        self.journal
            .iter()
            .any(|r| matches!(r, JournalRecord::OwnSeal(_) | JournalRecord::PeerLog(_)))
    }
}

/// A ceremony reconstructed from its journal by [`DkgCeremony::resume`]: the
/// rebuilt ceremony + the messages to re-emit. Seal-state is NOT returned as a
/// flag — the actor derives it from the ceremony itself ([`DkgCeremony::dealing_closed`]
/// = dealing closed; [`DkgCeremony::own_log_recorded`] = own-log-recorded/sealed-for-
/// finalize), so a torn-own-seal resume needs no special bit.
///
/// Resume has TWO shapes, chosen by the actor's `reconstruct_dealer` TIMING GATE
/// (`height < seal_deadline`, §8.11.1):
/// - **PRE-seal (reconstruct):** the dealer is RE-DERIVED via the deterministic
///   [`dealer_seed_rng`] seed, so it reproduces the byte-identical commitment + shares
///   (idempotent re-deal — no self-equivocation). The node keeps a LIVE dealer, resumes
///   BOTH delivery legs (retransmit un-acked dealings + the player ack-cache), replays
///   the acks its dealer already collected (journaled [`JournalRecord::OwnDealerAck`]),
///   feeds its OWN self-ack so the restarted dealer does NOT reveal itself, and seals
///   ONCE at the deadline. This is provably safe: `height < seal_deadline ⇒ we never
///   sealed ⇒ no original log was ever broadcast`, so re-deriving is idempotent-first.
/// - **At/after the seal deadline (player-only):** the dealer role is RETIRED — a torn
///   journal cannot prove we did not already seal + broadcast a possibly-divergent log,
///   so we NEVER re-seal. `outgoing` carries (a) one [`DkgBody::Ack`] per peer dealing
///   replayed by [`Player::resume`] (the resolver cannot heal a missing ack), and (b)
///   iff we had already sealed, our journaled own log re-broadcast VERBATIM. A node that
///   crashed BEFORE sealing sits out as one of the ≤f tolerated absent dealers — the
///   ceremony still finalizes on the n−f survivors and we recover our SHARE as a player.
pub struct Resumed {
    pub ceremony: DkgCeremony,
    pub outgoing: Vec<Outgoing>,
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
    /// Distinct dealers whose log has been recorded into `logs` (our own at seal +
    /// each peer's `Reveal`). Lets the supervisor's deterministic-settle gate detect
    /// "all `n` logs in" so every honest node finalizes over the IDENTICAL set.
    recorded: BTreeSet<PeerPubkey>,
    /// The SIGNED form of each recorded log (`logs` only retains the unsigned
    /// `DealerLog` post-`check`). Retained so the actor can SERVE peers a
    /// `SignedDealerLog` on a DKG-log recovery fetch (`signed_log`) and RE-BROADCAST
    /// our own log verbatim on a resume — both need the signature, which `Logs`
    /// discards.
    signed_logs: BTreeMap<PeerPubkey, DealerReveal>,
    /// Our own dealer commitment, retained so [`retransmit`](Self::retransmit) can
    /// re-send it point-to-point to any player still in `unsent` (the DEALER LEG of
    /// reliable delivery). `Some` while we have a live dealer (start / pre-seal resume),
    /// `None` on a player-only resume.
    own_pub_msg: Option<DealerCommitment>,
    /// Players who have NOT yet acked our dealing → the retransmit targets, holding the
    /// private share we owe each. Pruned as acks land (`handle(Ack)`), so it SHRINKS to
    /// ∅; retransmit is bounded by it.
    unsent: BTreeMap<PeerPubkey, DealerPrivMsg>,
    /// The ack WE emitted as a PLAYER, keyed by dealer — the ack re-emit cache (the
    /// PLAYER LEG of reliable delivery). On a re-receipt of a dealing we already acked
    /// (its dealer redelivered because our prior ack was lost), `try_ack` re-emits the
    /// cached ack instead of dropping it, so the ack retries on the dealer's retransmit
    /// cadence until it lands.
    emitted_acks: BTreeMap<PeerPubkey, Ack>,
}

/// Build the commonware DKG [`Info`] for `epoch` over `committee` — the SINGLE
/// place the ceremony's fixed parameters (`N3f1`, `Mode::NonZeroCounter`, dealers ==
/// players == `committee`) are pinned, so `start`/`resume`/the disk-backed log
/// re-check can never drift on them. `pub(crate)` so the actor's post-boundary
/// recompute-heal (`beacon::actor`) can re-`check` one delivered log against the
/// SAME pinned `Info` before journaling it.
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

/// Re-`check` a finalized epoch's journaled logs into a `dealer → SignedDealerLog`
/// serve map — the cold-load path the actor's `serve_cache` takes on a miss after a
/// restart (the journal is the durable source; this rebuilds the in-memory copy). Each
/// `OwnSeal`/`PeerLog` record is `check`-verified against the epoch's `Info` (cheap
/// once-per-epoch, bounding a tampered on-disk journal a node would otherwise serve to
/// peers); records that fail `check` are dropped. A bad committee/namespace surfaces as
/// `Err` so the caller declines to serve (retry-elsewhere), never serving un-verifiable
/// logs.
pub fn checked_serve_map(
    namespace: &[u8],
    epoch: u64,
    committee: Set<PeerPubkey>,
    records: Vec<JournalRecord>,
) -> Result<BTreeMap<PeerPubkey, DealerReveal>, DkgError> {
    let info = info_for(namespace, epoch, committee)?;
    let mut map = BTreeMap::new();
    for record in records {
        let signed = match record {
            JournalRecord::OwnSeal(signed) | JournalRecord::PeerLog(signed) => *signed,
            JournalRecord::ReceivedDealing(..) | JournalRecord::OwnDealerAck(..) => continue,
        };
        if let Some((pk, _)) = signed.clone().check(&info) {
            map.insert(pk, signed);
        }
    }
    Ok(map)
}

/// The namespace domain-separating the dealer-polynomial seed derivation.
///
/// SECRECY-CRITICAL: the intermediate signature this seeds from fully determines the
/// dealer polynomial (hence the whole share) — it MUST NEVER be logged or transmitted.
const DEALER_SEED_NS: &[u8] = b"FLUENT_DPOS_DKG_DEALER_SEED_V1";

/// Deterministic dealer-polynomial RNG derived from the validator key + epoch, so
/// re-creating the dealer after a restart reproduces the byte-identical commitment +
/// shares (idempotent re-deal — no self-equivocation, §8.11.1). Standard Ed25519
/// signing (RFC 8032) is deterministic ⇒ stable per `(me_key, epoch)`; it needs the
/// secret key ⇒ the seed is secret; nothing is persisted ⇒ it survives datadir loss
/// (unlike the former `OsRng` re-deal, which diverged across restarts). The idempotency
/// + ed25519-determinism are pinned by the `seed_*` unit tests.
fn dealer_seed_rng(me_key: &Ed25519PrivateKey, epoch: u64) -> impl CryptoRngCore {
    let sig = me_key.sign(DEALER_SEED_NS, &epoch.to_be_bytes());
    let mut t = Transcript::new(DEALER_SEED_NS);
    t.commit(sig.as_ref());
    Transcript::resume(t.summarize()).noise(b"dealer-rng")
}

/// The DEALER setup that both [`DkgCeremony::start`] and the pre-seal reconstruct
/// branch of [`DkgCeremony::resume`] run IDENTICALLY (the loop-break, §8.11.1): derive
/// the seeded RNG, `Dealer::start`, and split the private shares into our OWN (returned
/// separately for the self-ack + journal) and the per-player `unsent` retransmit set
/// (every OTHER player). Extracted so the dealer polynomial + `unsent` can never diverge
/// or be half-populated between the two paths. The self-ack differs only in SOURCE
/// (`start` mints it fresh; `resume` replays it from `Player::resume`), so it is fed by
/// the caller, not here.
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
    /// Begin a fresh ceremony for `epoch` over `committee` (Model B: dealers ==
    /// players == `committee`). The dealer polynomial is derived DETERMINISTICALLY from
    /// the validator key + epoch ([`dealer_seed_rng`]), so a restart re-derives the
    /// IDENTICAL commitment + shares (no `OsRng` divergence / self-equivocation).
    /// Returns the initial outgoing messages: this node's commitment (broadcast) + one
    /// private share per OTHER player; this node's own dealing is fed into its own
    /// player/dealer locally. The [`Step`]'s journal carries our self-dealing as a
    /// `ReceivedDealing` so a resume rebuilds `Player.view[me]`.
    pub fn start(
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
    ) -> Result<(Self, Step), DkgError> {
        let me = me_key.public_key();
        let info = info_for(namespace, epoch, committee)?;
        let mut player = Player::new(info.clone(), me_key.clone())?;
        let (mut dealer, pub_msg, unsent, self_priv) = init_dealer(&me_key, epoch, &info)?;

        // Self-dealing: process our own commitment+share locally so our player counts it
        // and our dealer collects its own self-ack (else the dealer would REVEAL its own
        // point at seal, burning an f-slot). Cache the self-ack (symmetry with the
        // resume reconstruct path, which re-derives it from `Player::resume`).
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
                recorded: BTreeSet::new(),
                signed_logs: BTreeMap::new(),
                own_pub_msg: Some(pub_msg),
                unsent,
                emitted_acks,
            },
            step,
        ))
    }

    /// Handle one incoming ceremony message from peer `from`. Invalid messages are
    /// dropped (the commonware primitives validate internally); returns the outgoing
    /// messages this triggers (an ack on a complete dealing) AND the journal records
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
                // Prune the acking player from our retransmit set and journal the ack so
                // a pre-seal-reconstruct resume can replay it (else the restarted dealer
                // would re-reveal an already-acked player). A wrong-sig ack from a
                // Byzantine peer still prunes — it only self-harms (≤ f); honest acks pass
                // `receive_player_ack`'s sig-verify.
                if self.unsent.remove(&from).is_some() {
                    step.journal
                        .push(JournalRecord::OwnDealerAck(from, Box::new(ack)));
                }
                step
            }
            DkgBody::Reveal(signed) => {
                let mut step = Step::default();
                if let Some((pk, log)) = (*signed).clone().check(&self.info) {
                    step.journal = self.record_checked_log(pk, log, *signed);
                }
                step
            }
        }
    }

    /// Record a `check`-valid log under `pk` if not already recorded, returning the
    /// journal records to append — EMPTY on a duplicate (the dedup that bounds journal
    /// growth + fsync churn). Shared by the gossip-Reveal path ([`handle`](Self::handle))
    /// and the resolver-ingest path ([`ingest_signed_log`](Self::ingest_signed_log)) so
    /// both grow the journal symmetrically under one dedup rule.
    fn record_checked_log(
        &mut self,
        pk: PeerPubkey,
        log: DealerLog<MinSig, PeerPubkey>,
        signed: DealerReveal,
    ) -> Vec<JournalRecord> {
        if self.recorded.insert(pk.clone()) {
            self.logs.record(pk.clone(), log);
            self.signed_logs.insert(pk, signed.clone());
            vec![JournalRecord::PeerLog(Box::new(signed))]
        } else {
            Vec::new()
        }
    }

    /// If both the commitment and private share from `dealer_pk` are buffered,
    /// process the dealing and emit an ack (point-to-point) if the player accepts.
    /// On accept, the dealing entered `Player.view`, so journal it (`ReceivedDealing`)
    /// — the resume must re-feed it — and CACHE the emitted ack.
    ///
    /// On a RE-RECEIPT (the dealer redelivered a dealing we already acked — our prior
    /// ack was likely lost — so `Player::dealer_message` returns `None`, view already
    /// holds it), RE-EMIT the cached ack instead of dropping it (the PLAYER LEG of
    /// reliable delivery). No new `ReceivedDealing` journal (already durable ⇒ the
    /// empty-journal `Step` passes the write-before-ack gate trivially).
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

    /// Re-send each un-acked dealing (`Commitment` + `Share`) point-to-point while the
    /// dealing phase is open — the DEALER LEG of reliable delivery (§8.11.1). A member
    /// that missed our initial dealing (or whose ack we lost) receives it again and
    /// (re-)acks; `unsent` SHRINKS to ∅ as acks land (`handle(Ack)` prunes it), so this
    /// converges in ~1–2 ticks. A no-op once the ceremony has sealed or resumed
    /// player-only (`dealing_closed()` ⇒ no dealer). BOTH halves are sent — a missed
    /// player missed both, and the receiver's `try_ack` needs the pair to ack.
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

    /// Close the dealing phase: finalize this node's dealer into a signed log,
    /// record it locally, and broadcast it as a `Reveal` so every player can
    /// include it. The [`Step`] journal carries our `OwnSeal` log so a resume
    /// re-broadcasts THAT log (no divergent re-deal). Idempotent — a second call is a
    /// no-op (dealer already taken).
    ///
    /// The `OwnSeal` record + the `Reveal` broadcast are produced ONLY when our own
    /// freshly-finalized log `check`s against our own `Info` — mirroring the resume
    /// side, which only marks the epoch sealed after `check` succeeds. Journaling an
    /// `OwnSeal` whose log is absent from `signed_logs` would let a resume mark the
    /// epoch sealed with no log to re-broadcast (a silent absent/stall); broadcasting
    /// an un-`check`able Reveal only spends bandwidth on a log peers reject.
    pub fn seal_dealings(&mut self) -> Step {
        let Some(dealer) = self.dealer.take() else {
            return Step::default();
        };
        let signed = dealer.finalize::<N3f1>();
        let mut step = Step::default();
        if let Some((pk, log)) = signed.clone().check(&self.info) {
            self.recorded.insert(pk.clone());
            self.logs.record(pk.clone(), log);
            self.signed_logs.insert(pk, signed.clone());
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
            // Our OWN freshly-finalized dealer log failed self-`check` (a library/encoding
            // edge or `Info`/variant mismatch). Emitting no `OwnSeal`/`Reveal` is correct
            // (above), but otherwise SILENT — surface it so the fault is visible instead of
            // the node quietly self-removing from the dealer quorum with no diagnostic
            // (review [3150]).
            tracing::warn!(
                epoch = self.epoch,
                "live DKG: own sealed dealer log failed self-check — no OwnSeal/Reveal emitted; \
                 this node contributes no dealing this epoch (recovers its share as a player)"
            );
        }
        step
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

    /// Count of DISTINCT dealer logs recorded so far (our own at seal + each peer's
    /// `Reveal`). When this equals the committee size, every reachable log is in;
    /// combined with [`ready`](Self::ready) (a valid quorum is selectable) it is the
    /// supervisor's "all-in" signal — every honest node then holds the IDENTICAL log
    /// set, so the deterministic `select` derives the identical `PK_E`.
    pub fn recorded_log_count(&self) -> usize {
        self.recorded.len()
    }

    /// The set of dealers whose log this ceremony has recorded — diffed against the
    /// committee roster to enumerate the missing dealer keys a shorthanded ceremony
    /// `fetch`es via the DKG-log recovery resolver.
    pub fn recorded_dealers(&self) -> &BTreeSet<PeerPubkey> {
        &self.recorded
    }

    /// The dealing phase is closed (we sealed — normally OR check-failed — or resumed
    /// player-only). The dealer `Option` is the durable in-ceremony seal state: it is
    /// `take`n at [`seal_dealings`](Self::seal_dealings) BEFORE its `check`. A PRE-seal
    /// [`resume`](Self::resume) (reconstruct) keeps a LIVE dealer (`Some`); an at/after-
    /// deadline resume is player-only (`None`). The actor derives seal-suppression /
    /// stop-buffering from this instead of a duplicated `sealed` flag.
    pub fn dealing_closed(&self) -> bool {
        self.dealer.is_none()
    }

    /// Our own valid log is recorded — the seal-before-finalize precondition AND the
    /// torn-own-seal recovery target. `seal_dealings` inserts `me` into `recorded` at
    /// the instant of a successful seal; a torn-own-seal node makes this true again the
    /// moment the resolver re-fetches its own log. The actor's finalize gate derives
    /// from this, so a check-failed seal (`me ∉ recorded`) correctly does NOT finalize
    /// over a set missing its own valid log.
    pub fn own_log_recorded(&self, me: &PeerPubkey) -> bool {
        self.recorded.contains(me)
    }

    /// The SIGNED form of one recorded dealer log, if held — served to a peer over
    /// the DKG-log recovery resolver (`log_resolver::Producer`) and re-broadcast for
    /// our own on resume. Each is `check`-valid by construction (only checked logs
    /// are inserted), so a served log always re-verifies on the requester.
    pub fn signed_log(&self, dealer: &PeerPubkey) -> Option<&DealerReveal> {
        self.signed_logs.get(dealer)
    }

    /// Take this ceremony's recorded signed logs, leaving it with an empty map. The
    /// actor calls this immediately BEFORE [`finalize`](Self::finalize) (which
    /// consumes `self`) to eagerly seed the finalized epoch's `serve_cache` (a bounded
    /// subset-copy of the journal), so the DKG-log recovery `Producer` can keep serving
    /// them to a late-restarting peer until the past-boundary sweep — an O(1) lookup
    /// with no disk read / no per-request `check` on the actor's hot path.
    pub fn take_signed_logs(&mut self) -> BTreeMap<PeerPubkey, DealerReveal> {
        std::mem::take(&mut self.signed_logs)
    }

    /// Re-`check` and record one `SignedDealerLog` recovered via the resolver
    /// (`log_resolver::Consumer`) for the fetched `expected` dealer. The log must
    /// both `check`-verify AND be signed by `expected` — a peer that answers a
    /// targeted fetch for D with a valid log for a DIFFERENT dealer D' must NOT
    /// satisfy the D fetch (and the log is not recorded under this fetch).
    /// Returns `(accepted, journal)`:
    /// - `accepted == true` — the log `check`-verified as `expected`'s log (now
    ///   recorded, or an honest duplicate already held);
    /// - `accepted == false` — `check` failed (a forgery) OR the log was signed by a
    ///   dealer ≠ `expected` (a mis-targeted answer). The caller returns the resolver
    ///   `deliver→false` (block the peer + re-fetch the key).
    ///
    /// The journal record (`PeerLog`) to persist is returned alongside (empty when
    /// the log was a duplicate or rejected), so the caller can append it.
    pub fn ingest_signed_log(
        &mut self,
        expected: &PeerPubkey,
        signed: DealerReveal,
    ) -> (bool, Vec<JournalRecord>) {
        match signed.clone().check(&self.info) {
            Some((pk, _)) if pk != *expected => {
                // A valid log, but for a different dealer than the one fetched — do
                // NOT record it under this fetch; the resolver re-fetches `expected`.
                (false, Vec::new())
            }
            Some((pk, log)) => {
                // Valid + correctly-targeted: record (deduped) and accept. An honest
                // duplicate (already recorded) returns `(true, [])` — valid, no journal.
                (true, self.record_checked_log(pk, log, signed))
            }
            None => (false, Vec::new()),
        }
    }

    /// Reconstruct a ceremony for `epoch` from its on-disk journal after a restart
    /// (§8.11.1). Rebuilds `Player.view` via [`Player::resume`] over the journaled
    /// received dealings + recorded logs, repopulates `Logs`/`recorded`/`signed_logs`,
    /// and RE-EMITS one [`DkgBody::Ack`] per PEER dealing [`Player::resume`] replayed
    /// (the resolver cannot heal a missing ack, so a peer must still be able to seal).
    ///
    /// `reconstruct_dealer` (the actor's TIMING GATE, `height < seal_deadline`) selects
    /// the resume shape:
    /// - **`true` (PRE-seal):** RE-DERIVE the dealer via the deterministic
    ///   [`init_dealer`]/[`dealer_seed_rng`] seed (byte-identical commitment + shares ⇒
    ///   idempotent, never self-equivocating), repopulate `own_pub_msg`/`unsent`, replay
    ///   the acks our dealer already collected (journaled [`JournalRecord::OwnDealerAck`]
    ///   → `receive_player_ack` + prune `unsent`), and FEED OUR OWN SELF-ACK from
    ///   `Player::resume`'s `acks[me]` (`Dealer::start` seeds `results[me] = Reveal`; only
    ///   a self-ack flips it, else the restarted dealer reveals ITSELF and burns an
    ///   f-slot). The node keeps a LIVE dealer, resumes BOTH delivery legs, and seals ONCE
    ///   at the deadline. Safe because `height < seal_deadline ⇒ we never sealed ⇒ no
    ///   original log was ever broadcast` (idempotent-first). On a `Dealer::start`/seed
    ///   error we degrade to player-only rather than fail the whole resume.
    /// - **`false` (at/after the deadline):** PLAYER-ONLY — the dealer role is RETIRED and
    ///   we NEVER re-seal (a torn journal cannot prove we did not already seal + broadcast
    ///   a possibly-divergent log). If a valid `OwnSeal` record is present we re-broadcast
    ///   THAT exact log verbatim; otherwise we sit out as one of the ≤f tolerated absent
    ///   dealers and recover our SHARE as a player over the n−f survivors.
    ///
    /// The peer acks in `Player::resume`'s map are ALWAYS re-emitted outbound (`!= me`);
    /// only the `me` entry is redirected to our own reconstructed dealer, so no peer ack
    /// is ever dropped or double-handled. `emitted_acks` is rebuilt from that map in BOTH
    /// shapes so the player ack-cache re-emit works after any resume.
    ///
    /// `MissingPlayerDealing` (a truncated journal dropped a publicly-acked dealing)
    /// surfaces as `Err` so the caller can sit out the epoch gracefully — never crash.
    pub fn resume(
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
        records: Vec<JournalRecord>,
        reconstruct_dealer: bool,
    ) -> Result<Resumed, DkgError> {
        let me = me_key.public_key();
        let info = info_for(namespace, epoch, committee)?;

        let mut log_map: BTreeMap<PeerPubkey, DealerLog<MinSig, PeerPubkey>> = BTreeMap::new();
        let mut signed_logs: BTreeMap<PeerPubkey, DealerReveal> = BTreeMap::new();
        let mut own_seal = false;
        // (dealer, pub, priv) dealings to feed `Player::resume` (incl. our own
        // self-dealing — it rebuilds `view[me]` AND regenerates our self-ack).
        let mut received: Vec<(PeerPubkey, DealerCommitment, DealerPrivMsg)> = Vec::new();
        // Acks our OWN dealer collected from players before the crash (journaled by
        // `handle(Ack)`) — replayed into the reconstructed dealer so it does not re-reveal
        // an already-acked player. Ignored on the player-only shape (no live dealer).
        let mut own_dealer_acks: Vec<(PeerPubkey, Ack)> = Vec::new();
        for record in records {
            match record {
                JournalRecord::ReceivedDealing(dealer, pub_msg, priv_msg) => {
                    received.push((dealer, *pub_msg, *priv_msg));
                }
                // Mark `own_seal` only AFTER `check` succeeds and populates
                // `signed_logs[me]` — a tampered `OwnSeal` frame that fails `check`
                // must NOT mark the epoch sealed (which would leave us with no log to
                // re-broadcast → a silent absent/stall).
                JournalRecord::OwnSeal(signed) => {
                    if let Some((pk, log)) = (*signed).clone().check(&info) {
                        own_seal = true;
                        log_map.insert(pk.clone(), log);
                        signed_logs.insert(pk, *signed);
                    }
                }
                JournalRecord::PeerLog(signed) => {
                    if let Some((pk, log)) = (*signed).clone().check(&info) {
                        log_map.insert(pk.clone(), log);
                        signed_logs.insert(pk, *signed);
                    }
                }
                JournalRecord::OwnDealerAck(player, ack) => {
                    own_dealer_acks.push((player, *ack));
                }
            }
        }

        // Rebuild `Player.view` and capture the regenerated per-dealer acks
        // (`Player::resume` re-emits one ack for every dealing it replays, INCLUDING our
        // own self-ack under `me`).
        let (player, acks) =
            Player::resume::<N3f1>(info.clone(), me_key.clone(), &log_map, received)?;

        let mut logs = Logs::<MinSig, PeerPubkey, N3f1>::new(info.clone());
        let mut recorded = BTreeSet::new();
        for (pk, log) in log_map {
            recorded.insert(pk.clone());
            logs.record(pk, log);
        }

        // Player ack-cache (BOTH shapes): every dealer we validly acked as a player, so a
        // later re-receipt of that dealing re-emits the cached ack (`try_ack`'s re-receipt
        // arm) — the player leg stays reliable after a restart too.
        let emitted_acks: BTreeMap<PeerPubkey, Ack> =
            acks.iter().map(|(d, a)| (d.clone(), a.clone())).collect();

        // Re-emit a point-to-point Ack to every PEER dealer whose dealing we replayed, so
        // a peer that lost our prior ack can still seal its log. The `me` self-ack is NOT
        // networked — it feeds our OWN reconstructed dealer below (or is moot player-only).
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
            // PRE-seal restart: re-derive the identical (seeded) dealer + resume both legs.
            match init_dealer(&me_key, epoch, &info) {
                Ok((mut dealer, pub_msg, mut unsent, _self_priv)) => {
                    // Replay the acks our dealer had already collected: flip each player's
                    // Reveal→Ack and prune it from the retransmit set.
                    for (player_pk, ack) in own_dealer_acks {
                        let _ = dealer.receive_player_ack(player_pk.clone(), ack);
                        unsent.remove(&player_pk);
                    }
                    // Feed our OWN self-ack so the restarted dealer does NOT reveal itself
                    // (`Dealer::start` seeds `results[me] = Reveal`; only this flips it).
                    if let Some(self_ack) = acks.get(&me) {
                        let _ = dealer.receive_player_ack(me.clone(), self_ack.clone());
                        unsent.remove(&me);
                    }
                    (Some(dealer), Some(pub_msg), unsent)
                }
                Err(e) => {
                    // Degenerate (Model B guarantees `me` deals). Degrade to player-only
                    // rather than sit the epoch out entirely — the n−f survivors finalize
                    // and we recover our share as a player.
                    tracing::warn!(
                        epoch,
                        ?e,
                        "live DKG: dealer reconstruct on resume failed — falling back to player-only"
                    );
                    (None, None, BTreeMap::new())
                }
            }
        } else {
            // At/after the seal deadline: PLAYER-ONLY, NEVER re-seal. If we sealed before
            // the crash, re-broadcast our journaled log verbatim so any peer that missed
            // it records OURS.
            if own_seal {
                if let Some(signed) = signed_logs.get(&me) {
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

        Ok(Resumed {
            ceremony: Self {
                epoch,
                info,
                dealer,
                player: Some(player),
                logs,
                pending_pub: BTreeMap::new(),
                pending_priv: BTreeMap::new(),
                recorded,
                signed_logs,
                own_pub_msg,
                unsent,
                emitted_acks,
            },
            outgoing,
        })
    }

    /// Whether this ceremony can still attempt [`finalize`](Self::finalize) — i.e. its
    /// `Player` has not already been consumed by a prior finalize. A finalize-`Err`
    /// (transient `MissingPlayerDealing` race, see [`finalize`](Self::finalize))
    /// consumes the player; the supervisor's finalize gate derives from this so the
    /// ceremony is NOT re-pulled into a destructive finalize, yet stays in the map to
    /// keep SERVING its recorded logs to recovering peers until the boundary sweep.
    pub fn can_finalize(&self) -> bool {
        self.player.is_some()
    }

    /// Derive the agreed [`CeremonyOutput`] (`PK_E`) + this node's secret [`Share`]
    /// over the collected logs. NON-DESTRUCTIVE to the ceremony object: it borrows
    /// `&mut self` and CONSUMES only the `Player` (the commonware `Player::finalize`
    /// takes it by value either way), leaving the recorded `logs`/`recorded`/`signed_logs`
    /// intact so the ceremony keeps serving peers.
    ///
    /// CALLER CONTRACT (the supervisor enforces timing):
    /// - [`seal_dealings`](Self::seal_dealings) MUST have run first — it is the only
    ///   place this node's own dealer log enters `self.logs` and is broadcast, so
    ///   finalizing without sealing drops this dealer from the quorum (locally AND for
    ///   every peer).
    /// - At least a dealer-quorum of VALID logs must be recorded, else
    ///   `Player::finalize` returns `Err(DkgFailed)`.
    ///
    /// On `Err` the ceremony is NOT destroyed — the supervisor removes it from its map
    /// ONLY on `Ok`. A transient `MissingPlayerDealing` race (`observe`/`ready` returns
    /// `Ok` while `Player::finalize` over the just-`select`ed set returns `Err` because a
    /// freshly-resumed node's rebuilt `view` lags a delivered log) thus no longer forfeits
    /// the share by destroying the whole ceremony; the ceremony sits out gracefully
    /// ([`can_finalize`](Self::can_finalize) is now false, so the gate stops re-pulling it)
    /// while still serving its recorded logs. `self.logs` is cloned (cheap, `Logs: Clone`)
    /// so the recorded set survives.
    pub fn finalize<R: CryptoRngCore>(
        &mut self,
        rng: &mut R,
    ) -> Result<(CeremonyOutput, Share), DkgError> {
        let player = self.player.take().expect("can_finalize gates this");
        player.finalize::<N3f1, ed25519::Batch>(rng, self.logs.clone(), &Sequential)
    }

    /// The content hash `keccak256(encode(SignedDealerLog))` of the recorded log for
    /// `dealer`, if held — the deterministic INDEX over the recorded bodies that
    /// rides `OrderBlock.dkg_logs` (AMENDMENT 5). `None` if not recorded.
    pub fn signed_log_hash(&self, dealer: &PeerPubkey) -> Option<B256> {
        self.signed_logs.get(dealer).map(|s| keccak256(s.encode()))
    }

    /// Build a `Logs` restricted to EXACTLY the finalized-consensus pinned dealer-log
    /// set (AMENDMENT 5 determinism core): for each `(idx, hash)` in `pinned` map
    /// `idx → committee[idx]` (position order — the agreed on-chain committee), and
    /// include our recorded signed log for that dealer IFF its CONTENT HASH matches
    /// `hash` (content-addressed: a mismatched/absent body is DROPPED and
    /// `all_held=false`, so every honest node fetches the SAME pinned bytes and
    /// selects over the IDENTICAL set → identical `PK_E`). Returns `(scoped_logs,
    /// all_held)`; `all_held=false` ⇒ a pinned body is MISSING OR MISMATCHED (the
    /// caller WAITS, never subset-finalizes — the resolver fetches it).
    ///
    /// An idx with no position in `committee` is a different case and is SKIPPED, not
    /// counted against `all_held`: nothing can ever satisfy it, because the resolver
    /// fetches per-DEALER over the roster and there is no dealer to name. Holding
    /// `all_held=false` for it wedged the ceremony forever, silently — the deep half
    /// of the 2026-07-21 idx-stall. The skip stays deterministic because it is a pure
    /// function of two pieces of agreed data (the consensus-pinned map and the
    /// committed committee), so every honest node scopes the IDENTICAL set and derives
    /// the IDENTICAL `PK_E`. If the mappable remainder cannot reach quorum the caller
    /// falls into the existing below-quorum path instead of hanging. The actor counts
    /// and logs the occurrence (`dpos_dkg_pinned_idx_out_of_range_total`); with the
    /// codec bound in `order_block.rs` an honest chain can no longer produce one.
    fn scoped_pinned_logs(
        &self,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> (Logs<MinSig, PeerPubkey, N3f1>, bool) {
        let mut logs = Logs::<MinSig, PeerPubkey, N3f1>::new(self.info.clone());
        let mut all_held = true;
        for (idx, hash) in pinned {
            let Some(pk) = committee.iter().nth(*idx as usize) else {
                continue; // unmappable idx — deterministic skip, see the docstring
            };
            match self.signed_logs.get(pk) {
                Some(signed) if keccak256(signed.encode()) == *hash => {
                    // Re-`check` (guaranteed to pass — only checked logs are stored)
                    // to obtain the `DealerLog` for `Logs::record`.
                    if let Some((cpk, log)) = signed.clone().check(&self.info) {
                        logs.record(cpk, log);
                    } else {
                        all_held = false;
                    }
                }
                _ => all_held = false, // missing OR a different body than the pinned hash
            }
        }
        (logs, all_held)
    }

    /// Non-destructive AM5 finalize probe over the PINNED set: `(ready, all_held)`.
    /// `ready` = a selectable quorum exists WITHIN the pinned+held dealers (`observe`
    /// over the scoped `Logs`, the SAME quorum logic as [`ready`](Self::ready));
    /// `all_held` = every MAPPABLE pinned body is held with a matching hash (fetch-
    /// before-finalize; unmappable indices are skipped, see
    /// [`scoped_pinned_logs`](Self::scoped_pinned_logs)). Finalize only when
    /// `all_held && ready` (or the all-`n` fast path).
    pub fn pinned_ready<R: CryptoRngCore>(
        &self,
        rng: &mut R,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> (bool, bool) {
        let (logs, all_held) = self.scoped_pinned_logs(committee, pinned);
        let ready =
            observe::<MinSig, PeerPubkey, N3f1, ed25519::Batch>(rng, logs, &Sequential).is_ok();
        (ready, all_held)
    }

    /// AM5 deterministic finalize: `Player::finalize` over EXACTLY the pinned set
    /// ([`scoped_pinned_logs`](Self::scoped_pinned_logs)). Consumes the player like
    /// [`finalize`](Self::finalize). The caller MUST have confirmed `all_held &&
    /// ready` via [`pinned_ready`](Self::pinned_ready) — scoping `select` to the
    /// finalized-consensus set is what makes the derived `PK_E` a pure function of
    /// agreed data (honest divergence impossible by construction).
    pub fn finalize_over_pinned<R: CryptoRngCore>(
        &mut self,
        rng: &mut R,
        committee: &Set<PeerPubkey>,
        pinned: &BTreeMap<u8, B256>,
    ) -> Result<(CeremonyOutput, Share), DkgError> {
        let (logs, _all_held) = self.scoped_pinned_logs(committee, pinned);
        let player = self.player.take().expect("can_finalize gates this");
        player.finalize::<N3f1, ed25519::Batch>(rng, logs, &Sequential)
    }
}

/// LOCALLY recompute this node's share for a FINALIZED epoch from its durable
/// journal, scoped to EXACTLY the pinned `dealers` set — the demote-heal primitive
/// (§8.11.1). Reuses the SAME `Player::resume` (rebuild `view` from the journaled
/// `ReceivedDealing`s) + `Player::finalize` the mid-window-restart path runs, but
/// selects over ONLY the `dealers`-scoped logs: a SUPERSET would make `Logs::select`
/// pick a different first-`n−f` → a different `PK_E` → the caller's self-check fails
/// though safety still holds (see `outcome::validate_share_on_poly`). Player-only —
/// never re-deals. The caller MUST self-verify the returned share against the pinned
/// `Output` before adopting; a short / torn / tampered journal fails that check (or
/// returns `Err` here) and is NEVER adopted, so a wrong-but-valid-looking share can
/// never leak into consensus. `rng` drives only `Player::finalize`'s batch verify.
///
/// The `dealers` scope is the pinned `Output::dealers()` (the qualified set the chain
/// agreed produced `PK_E`); a member's own point for each selected dealer comes from
/// its journaled `ReceivedDealing` (acked) OR that dealer's log reveal (un-acked), so
/// this recovers the share with no live ceremony.
pub fn recompute_scoped<R: CryptoRngCore>(
    rng: &mut R,
    namespace: &[u8],
    epoch: u64,
    committee: Set<PeerPubkey>,
    me_key: Ed25519PrivateKey,
    dealers: &Set<PeerPubkey>,
    records: Vec<JournalRecord>,
) -> Result<(CeremonyOutput, Share), DkgError> {
    let info = info_for(namespace, epoch, committee)?;
    let scope: BTreeSet<&PeerPubkey> = dealers.iter().collect();

    let mut log_map: BTreeMap<PeerPubkey, DealerLog<MinSig, PeerPubkey>> = BTreeMap::new();
    let mut received: Vec<(PeerPubkey, DealerCommitment, DealerPrivMsg)> = Vec::new();
    for record in records {
        match record {
            JournalRecord::ReceivedDealing(dealer, pub_msg, priv_msg) => {
                received.push((dealer, *pub_msg, *priv_msg));
            }
            // Keep ONLY logs whose dealer is in the pinned `dealers` scope — a
            // superset would diverge `select`.
            JournalRecord::OwnSeal(signed) | JournalRecord::PeerLog(signed) => {
                if let Some((pk, log)) = (*signed).clone().check(&info) {
                    if scope.contains(&pk) {
                        log_map.insert(pk, log);
                    }
                }
            }
            // The player-state rebuild does not consume dealer-side acks.
            JournalRecord::OwnDealerAck(..) => {}
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

    /// Drive N ceremonies to completion purely through the event API (start →
    /// exchange Outgoing → seal → exchange → finalize), then assert every node
    /// agreed on `PK_E` and that the resulting shares recover a verifiable seed —
    /// i.e. the networked event model reproduces `run_local_dkg`.
    #[test]
    fn ceremonies_agree_and_feed_a_verifiable_seed_via_events() {
        let mut rng = StdRng::seed_from_u64(11);
        let keys: Vec<Ed25519PrivateKey> = (0..5)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        // (sender, outgoing) — the sender is the `from` the receiver's handle needs.
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(ns, 0, committee.clone(), k.clone()).expect("start");
            let from = k.public_key();
            queue.extend(step.outgoing.into_iter().map(|o| (from.clone(), o)));
            ceremonies.insert(from, cer);
        }

        // Deliver until quiescent (commitments/shares → acks; acks → nothing).
        deliver_all(&mut ceremonies, &mut queue);

        // Deadline: every node seals its dealings (broadcast Reveal); deliver them.
        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        deliver_all(&mut ceremonies, &mut queue);

        // Finalize: every node derives the agreed output + its share.
        let mut outputs = Vec::new();
        let mut shares = BTreeMap::new();
        for (pk, mut cer) in ceremonies {
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
        let sig = recover_seed::<N3f1>(outputs[0].public(), &partials).expect("recover");
        assert!(
            verify_seed(pk0, &seed_ns, round, &sig),
            "seed from the event-driven DKG shares must verify against PK_E"
        );
    }

    /// Drive `n=5` ceremonies through start → exchange → seal → exchange so EVERY
    /// node holds ALL 5 recorded dealer logs; return `(committee, ceremonies)` for the
    /// AMENDMENT 5 pinned-finalize tests.
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

    /// Build the pinned `idx→hash` map (AM5) for a subset of committee positions,
    /// reading the content hash from `source`'s recorded logs (content-addressed, so
    /// every node's hash for a given dealer is identical).
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

    /// AMENDMENT 5 CORE DETERMINISM PIN (test a): two DIFFERENT nodes (different
    /// players, different p2p arrival orders) that finalize over the IDENTICAL pinned
    /// dealer-log set derive the IDENTICAL `PK_E` — the finalize is a pure function of
    /// the finalized set, so honest divergence is impossible by construction. Holds
    /// for the full set AND a proper subset (a different but still-agreed key).
    #[test]
    fn finalize_over_pinned_is_pure_in_the_finalized_set() {
        let mut rng = StdRng::seed_from_u64(101);
        let (committee, mut ceremonies) = run_to_all_sealed(101);
        let nodes: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        // n=5, N3f1 ⇒ dealer quorum n−f = 4; `select` picks the LOWEST `quorum` valid
        // dealers, so `{0,1,2,3,4}` and `{0,1,2,3}` select the same 4 (same key), while
        // `{1,2,3,4}` selects a DIFFERENT quorum (different key). Content hashes are
        // node-independent — read them once from node 0.
        let lo = pinned_for(&committee, &ceremonies[&nodes[0]], &[0, 1, 2, 3]);
        let hi = pinned_for(&committee, &ceremonies[&nodes[0]], &[1, 2, 3, 4]);

        // Each node's `pinned_ready` over a quorum set: all held + a quorum selectable.
        for pk in &nodes {
            let (ready, all_held) = ceremonies[pk].pinned_ready(&mut rng, &committee, &lo);
            assert!(ready && all_held, "quorum pinned set: ready + all held");
        }

        // Finalize node 0 and node 1 over the SAME pinned set → identical PK_E.
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

        // A DIFFERENT pinned dealer quorum is a different DKG but still deterministic
        // across nodes (node 2 and node 3 over `{1,2,3,4}` agree with each other).
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

    /// AMENDMENT 5 (test b): a node whose local bodies are a SUBSET of the pinned
    /// hashes reports `all_held=false` — the actor WAITS (never subset-finalizes)
    /// until the resolver fetches the missing pinned bytes. A hash that does NOT match
    /// any held body (a not-yet-fetched / equivocated body) is dropped.
    #[test]
    fn unmappable_pinned_idx_is_skipped_and_does_not_wedge() {
        let mut rng = StdRng::seed_from_u64(303);
        let (committee, mut ceremonies) = run_to_all_sealed(303);
        let nodes: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        let clean = pinned_for(&committee, &ceremonies[&nodes[0]], &[0, 1, 2, 3]);

        // An idx with no position in this committee: nothing can ever satisfy it, since
        // the resolver fetches per-DEALER over the roster. Holding `all_held=false` for
        // it kept the ceremony from EVER finalizing, silently — the deep half of the
        // 2026-07-21 idx-stall.
        let mut bogus = clean.clone();
        bogus.insert(committee.len() as u8, B256::repeat_byte(0xAB));

        for pk in &nodes {
            let (ready, all_held) = ceremonies[pk].pinned_ready(&mut rng, &committee, &bogus);
            assert!(
                ready && all_held,
                "an unmappable idx must be skipped, not block the finalize gate"
            );
        }

        // And the skip does not change the outcome: same key as without the bogus entry,
        // and the same key on every node (the skip is a pure function of agreed data).
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
        // Correct full pinned set ⇒ all held.
        let full = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3, 4]);
        let (_ready, all_held) = ceremonies[&node].pinned_ready(&mut rng, &committee, &full);
        assert!(all_held, "matching hashes ⇒ all held");
        // Corrupt one pinned hash (a body this node does NOT hold) ⇒ not all held.
        let mut missing = full.clone();
        missing.insert(4, B256::repeat_byte(0xEE));
        let (_ready, all_held) = ceremonies[&node].pinned_ready(&mut rng, &committee, &missing);
        assert!(
            !all_held,
            "a pinned hash with no matching held body ⇒ WAIT (all_held=false)"
        );
    }

    /// AMENDMENT 5 (test c): a pinned set BELOW the reconstruction threshold at the
    /// deadline is not `ready` (no selectable quorum) ⇒ the ceremony does not finalize
    /// ⇒ clean deferral. `≥ t` ⇒ ready.
    #[test]
    fn pinned_ready_false_below_threshold_true_at_quorum() {
        let mut rng = StdRng::seed_from_u64(303);
        let (committee, ceremonies) = run_to_all_sealed(303);
        let node = ceremonies.keys().next().unwrap().clone();
        // n=5, N3f1 ⇒ dealer quorum n−f = 4. Three pinned dealers ⇒ below quorum ⇒
        // NOT ready.
        let three = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2]);
        let (ready, all_held) = ceremonies[&node].pinned_ready(&mut rng, &committee, &three);
        assert!(all_held, "the three bodies ARE held");
        assert!(
            !ready,
            "below quorum ⇒ no selectable quorum ⇒ not ready (defer)"
        );
        // Four pinned dealers ⇒ at quorum ⇒ ready.
        let four = pinned_for(&committee, &ceremonies[&node], &[0, 1, 2, 3]);
        let (ready, _) = ceremonies[&node].pinned_ready(&mut rng, &committee, &four);
        assert!(ready, "at quorum ⇒ a quorum is selectable ⇒ ready");
    }

    /// The seeded dealer is DETERMINISTIC: two `start`s for the same `(key, epoch)`
    /// produce the byte-identical commitment (the idempotent-re-deal / anti-self-
    /// equivocation property), while a different epoch differs. ed25519 signing (the
    /// seed source) is deterministic too — asserted directly, since a non-deterministic
    /// signer backend would silently re-introduce the divergence this seed removes.
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

    /// `retransmit` re-sends only to UN-acked players and is a no-op once sealed: it
    /// starts non-empty (every peer un-acked at start), shrinks to ∅ as acks land, and
    /// returns nothing after `dealing_closed()`.
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
        // 3 other players × (Commitment + Share).
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

    /// BOTH delivery legs heal via retransmit + ack re-emit. With dealer-0's initial
    /// dealing to player-1 DROPPED (dealer-leg loss) AND player-2's ack to dealer-0
    /// DROPPED (ack-leg loss), the per-tick `retransmit` re-delivers to both un-acked
    /// players and player-2 re-emits its CACHED ack, so dealer-0's sealed log reveals
    /// NEITHER — reveals track genuine whole-window absence, not a single dropped packet.
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

        // Lossy initial delivery: drop dealer-0's Commitment+Share to player-1 (dealer
        // leg) and player-2's Ack to dealer-0 (ack leg).
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

        // Retransmit ticks (no further loss) heal BOTH legs.
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

        // Seal + inspect dealer-0's OWN log: it reveals neither p1 nor p2.
        let sealers: Vec<PeerPubkey> = ceremonies.keys().cloned().collect();
        for pk in sealers {
            let step = ceremonies.get_mut(&pk).expect("ceremony").seal_dealings();
            queue.extend(step.outgoing.into_iter().map(|o| (pk.clone(), o)));
        }
        let info = info_for(ns, 0, committee.clone()).expect("info");
        let own = ceremonies[&d0]
            .signed_log(&d0)
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

        // Deliver the reveals + finalize: every node still agrees on PK_E.
        deliver_all(&mut ceremonies, &mut queue);
        let outputs: Vec<CeremonyOutput> = ceremonies
            .into_values()
            .map(|mut c| c.finalize(&mut rng).expect("finalize").0)
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

    /// `deliver_all` but drops any `(from, to, body)` for which `drop` returns true —
    /// modelling per-leg packet loss (a dropped Commitment/Share = dealer leg, a dropped
    /// Ack = player leg). Broadcast fan-out applies the predicate per recipient.
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

    /// Drive a fresh 4-party ceremony to the point where node-0 has acked every
    /// dealer and SEALED, capturing node-0's journal records. Returns
    /// `(committee, key0, journal0)` — the inputs a resume test reconstructs node-0
    /// from.
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

    /// `deliver_all` but ALSO appends node-`me`'s journal records as it handles each
    /// message — so a resume test can reconstruct exactly what node-`me` would have
    /// persisted.
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

    /// Resume reproduces node-0's exact share: a node that journaled its received
    /// dealings + own seal + peer logs, then crashed, rebuilds the IDENTICAL
    /// `(PK_E, share)` via `Player::resume` (no divergent re-deal).
    #[test]
    fn resume_after_own_seal_reproduces_identical_share() {
        let (committee, key0, journal0) = run_to_node0_sealed(31);

        // Live node-0: rebuild + finalize from the same network.
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
        let (out_live, share_live) = live
            .ceremony
            .finalize(&mut rng_live)
            .expect("finalize live");

        // Resumed node-0: rebuild from the journal alone (a DIFFERENT rng seed for
        // the re-spawn would, under a re-deal, diverge — player-only restore makes it
        // irrelevant: no rng is consumed).
        let mut rng_res = StdRng::seed_from_u64(99);
        let mut resumed =
            DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, journal0, false)
                .expect("resume");
        assert!(resumed.ceremony.own_log_recorded(&me0));
        let (out_res, share_res) = resumed
            .ceremony
            .finalize(&mut rng_res)
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
    /// `MissingPlayerDealing` on resume (graceful sit-out, never a crash).
    #[test]
    fn resume_with_missing_acked_dealing_reports_missing_player_dealing() {
        let (committee, key0, mut journal0) = run_to_node0_sealed(42);
        // Drop one received PEER dealing (not our own self-deal) — its dealer's log
        // still records our ack, so resume must fail MissingPlayerDealing.
        let me0 = key0.public_key();
        let idx = journal0
            .iter()
            .position(|r| matches!(r, JournalRecord::ReceivedDealing(d, _, _) if *d != me0))
            .expect("a peer dealing");
        journal0.remove(idx);

        match DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, journal0, false) {
            Err(DkgError::MissingPlayerDealing) => {}
            Err(other) => panic!("expected MissingPlayerDealing, got {other:?}"),
            Ok(_) => panic!("a missing acked dealing must fail resume"),
        }
    }

    /// A PRE-deadline resume (reconstruct) RE-DERIVES the identical (seeded) dealer and
    /// keeps distributing — the INVERSE of the old player-only sit-out. With only its
    /// received dealings journaled (an early crash), the reconstructed dealer's `unsent`
    /// covers every peer, so `retransmit` re-sends, and the re-derived commitment is
    /// byte-identical to what the original `start` would have dealt (seeded idempotency).
    #[test]
    fn pre_deadline_resume_reconstructs_identical_dealer() {
        let (committee, key0, journal0) = run_to_node0_sealed(13);
        // Early-crash journal: keep ONLY our received dealings (view), drop logs/acks so
        // the reconstructed dealer's `unsent` is the full peer set.
        let received_only: Vec<JournalRecord> = journal0
            .into_iter()
            .filter(|r| matches!(r, JournalRecord::ReceivedDealing(..)))
            .collect();

        // The commitment the ORIGINAL `start` would deal (seeded — deterministic).
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

    /// R3: a reconstructed dealer that received EVERY player's ack (journaled
    /// `OwnDealerAck`s) plus its own self-ack seals a log with ZERO reveals — it does NOT
    /// reveal ITSELF (which would burn an f-slot). Without feeding the self-ack, `me`'s
    /// entry would stay a Reveal.
    #[test]
    fn reconstructed_dealer_does_not_reveal_itself() {
        let (committee, key0, journal0) = run_to_node0_sealed(53);
        let me0 = key0.public_key();
        // Pre-seal (strip OwnSeal); KEEP the OwnDealerAcks (all peers acked us).
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

    /// A resume re-emits one `Ack` per peer dealing it replayed (a peer that lost our
    /// prior ack can then still seal its log). The own-seal resume path
    /// also carries the re-broadcast own log.
    #[test]
    fn resume_reemits_peer_acks() {
        let (committee, key0, journal0) = run_to_node0_sealed(17);
        let me0 = key0.public_key();
        // Peer dealings node-0 replays = the non-self ReceivedDealing records.
        let peer_dealers: BTreeSet<PeerPubkey> = journal0
            .iter()
            .filter_map(|r| match r {
                JournalRecord::ReceivedDealing(d, _, _) if *d != me0 => Some(d.clone()),
                _ => None,
            })
            .collect();
        assert!(!peer_dealers.is_empty(), "node-0 acked at least one peer");

        let resumed =
            DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, journal0, false)
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

    /// P3 determinism (RED→GREEN): `recompute_scoped` is a DETERMINISTIC function of
    /// the pinned `Output::dealers()` set, independent of journal acquisition order or
    /// held supersets. Node-0's share recomputed from its journal (scoped to the pinned
    /// dealers) is byte-identical to its canonically-finalized share AND lies on the
    /// pinned poly; recomputing from a REVERSED journal yields the byte-identical share.
    /// This is what makes two honest nodes recompute the SAME share for one seat — the
    /// fork-safety determinism the heal rests on.
    #[test]
    fn recompute_scoped_is_deterministic_over_pinned_dealers() {
        // Two byte-identical journals (same seed ⇒ same ceremony) so we can both
        // finalize the canonical share AND recompute from a copy (journals are not
        // `Clone` — they hold secret `DealerPrivMsg`).
        let (committee, key0, journal_canon) = run_to_node0_sealed(71);
        let (_c2, _k2, journal_recompute) = run_to_node0_sealed(71);
        let (_c3, _k3, journal_reversed) = run_to_node0_sealed(71);
        let me0 = key0.public_key();

        // Canonical: resume + finalize the way the live actor does.
        let mut rng = StdRng::seed_from_u64(1);
        let mut canon = DkgCeremony::resume(
            b"FLUENT_DPOS_V1_test",
            0,
            committee.clone(),
            key0.clone(),
            journal_canon,
            false,
        )
        .expect("resume");
        let (outcome, canonical_share) = canon.ceremony.finalize(&mut rng).expect("finalize");

        // Recompute scoped to the pinned dealer set from an independent journal copy.
        let mut rng2 = StdRng::seed_from_u64(999); // a DIFFERENT rng — recompute is deterministic
        let (recomputed_out, recomputed_share) = recompute_scoped(
            &mut rng2,
            b"FLUENT_DPOS_V1_test",
            0,
            committee.clone(),
            key0.clone(),
            outcome.dealers(),
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
            validate_share_on_poly(&outcome, &committee, &recomputed_share),
            "the recomputed share self-verifies against the pinned Output"
        );

        // Acquisition-order independence: recompute from the SAME journal REVERSED
        // (a differently-acquired-but-equal set) yields the byte-identical share —
        // scoping to dealers() (not raw insertion order) is what makes it deterministic.
        let mut rev = journal_reversed;
        rev.reverse();
        let mut rng3 = StdRng::seed_from_u64(7);
        let (_out_r, share_rev) = recompute_scoped(
            &mut rng3,
            b"FLUENT_DPOS_V1_test",
            0,
            committee,
            key0,
            outcome.dealers(),
            rev,
        )
        .expect("recompute reversed");
        assert_eq!(
            share_rev.encode().as_ref(),
            canonical_share.encode().as_ref(),
            "recompute is independent of journal acquisition order (scoped to dealers())"
        );
        let _ = me0;
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
