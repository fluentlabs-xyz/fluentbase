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

use crate::beacon::dkg_msg::{DealerCommitment, DealerReveal, DkgBody, DkgMsg};
use crate::beacon::share_state::JournalRecord;
use commonware_cryptography::{
    bls12381::{
        dkg::{
            observe, Dealer, DealerLog, DealerPrivMsg, Error as DkgError, Info, Logs, Output, Player,
        },
        primitives::{group::Share, sharing::Mode, variant::MinSig},
    },
    ed25519::{self, PrivateKey as Ed25519PrivateKey},
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
/// rebuilt ceremony + the messages to re-broadcast. Seal-state is NOT returned as a
/// flag — the actor derives it from the ceremony itself ([`DkgCeremony::dealing_closed`]
/// = dealing closed; [`DkgCeremony::own_log_recorded`] = own-log-recorded/sealed-for-
/// finalize), so a torn-own-seal resume needs no special bit.
///
/// Restore is PLAYER-ONLY: the dealer role is RETIRED on resume — we never
/// re-`Dealer::start` (fresh `OsRng` randomness → a divergent commitment peers
/// ignore → a lost / self-equivocating contribution, §8.11.1). `outgoing` carries
/// (a) one [`DkgBody::Ack`] per peer dealing replayed by [`Player::resume`] —
/// without re-emitting these, the peer can never SEAL the log a recovery fetch
/// would later need (the resolver cannot heal a missing ack); and (b) iff we had
/// already sealed, our journaled own log re-broadcast VERBATIM so any peer that
/// missed it records OURS. A node that crashed BEFORE sealing simply sits out as
/// one of the ≤f tolerated absent dealers — the ceremony still finalizes on the
/// n−f survivors and we recover our SHARE as a player from their logs + our
/// snapshot, no re-deal.
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
}

/// Build the commonware DKG [`Info`] for `epoch` over `committee` — the SINGLE
/// place the ceremony's fixed parameters (`N3f1`, `Mode::NonZeroCounter`, dealers ==
/// players == `committee`) are pinned, so `start`/`resume`/the disk-backed log
/// re-check can never drift on them.
fn info_for(
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
            JournalRecord::ReceivedDealing(..) => continue,
        };
        if let Some((pk, _)) = signed.clone().check(&info) {
            map.insert(pk, signed);
        }
    }
    Ok(map)
}

impl DkgCeremony {
    /// Begin a fresh ceremony for `epoch` over `committee` (Model B: dealers ==
    /// players == `committee`). Returns the initial outgoing messages: this node's
    /// commitment (broadcast) + one private share per OTHER player; this node's own
    /// dealing is fed into its own player/dealer locally. The [`Step`]'s journal
    /// carries our self-dealing as a `ReceivedDealing` so a resume rebuilds
    /// `Player.view[me]`.
    pub fn start<R: CryptoRngCore>(
        rng: &mut R,
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
    ) -> Result<(Self, Step), DkgError> {
        let me = me_key.public_key();
        let info = info_for(namespace, epoch, committee)?;
        let mut player = Player::new(info.clone(), me_key.clone())?;
        let (mut dealer, pub_msg, priv_msgs) =
            Dealer::start::<N3f1>(rng, info.clone(), me_key.clone(), None)?;

        let mut step = Step::out(vec![Outgoing {
            target: Target::Broadcast,
            msg: DkgMsg {
                ceremony_epoch: epoch,
                body: DkgBody::Commitment(Box::new(pub_msg.clone())),
            },
        }]);
        for (pk, priv_msg) in priv_msgs {
            if pk == me {
                // Self-dealing: process our own commitment+share locally so our
                // player counts it and our dealer collects our self-ack. Journal it
                // (the resume must re-feed it to rebuild `view[me]`).
                if let Some(ack) =
                    player.dealer_message::<N3f1>(me.clone(), pub_msg.clone(), priv_msg.clone())
                {
                    let _ = dealer.receive_player_ack(me.clone(), ack);
                }
                step.journal.push(JournalRecord::ReceivedDealing(
                    me.clone(),
                    Box::new(pub_msg.clone()),
                    Box::new(priv_msg),
                ));
            } else {
                step.outgoing.push(Outgoing {
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
                recorded: BTreeSet::new(),
                signed_logs: BTreeMap::new(),
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
                if let Some(dealer) = self.dealer.as_mut() {
                    let _ = dealer.receive_player_ack(from, ack);
                }
                Step::default()
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
    /// — the resume must re-feed it.
    fn try_ack(&mut self, dealer_pk: PeerPubkey) -> Step {
        if !(self.pending_pub.contains_key(&dealer_pk) && self.pending_priv.contains_key(&dealer_pk))
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
            None => Step::default(),
        }
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
    /// `take`n at [`seal_dealings`](Self::seal_dealings) BEFORE its `check`, and is
    /// `None` after [`resume`](Self::resume). The actor derives seal-suppression /
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

    /// Reconstruct a ceremony for `epoch` from its on-disk journal after a restart,
    /// PLAYER-ONLY — never re-dealing (§8.11.1). Rebuilds `Player.view` via
    /// [`Player::resume`] over the journaled received dealings + recorded logs,
    /// repopulates `Logs`/`recorded`/`signed_logs`, and:
    /// - RE-EMITS one [`DkgBody::Ack`] per peer dealing [`Player::resume`] replayed
    ///   (the `acks` map it returns — `(dealer → ack)`): a peer cannot SEAL the log
    ///   a later recovery fetch would need without our ack first, and the resolver
    ///   cannot heal a missing ack, so dropping these would ship broken;
    /// - if a valid `OwnSeal` record is present (we sealed before the crash): keeps
    ///   NO live dealer and re-broadcasts THAT exact signed log (the network already
    ///   recorded it) — `sealed` is `true` in the returned [`Resumed`];
    /// - otherwise (crashed before seal): keeps NO dealer either and sits out as one
    ///   of the ≤f tolerated absent dealers — the ceremony finalizes on the n−f
    ///   survivors and we recover our SHARE as a player from their logs + our
    ///   snapshot. The DEALER ROLE IS RETIRED in BOTH branches — a fresh
    ///   `Dealer::start` here would draw new `OsRng` randomness → a commitment that
    ///   diverges from any our peers may already hold → a lost / self-equivocating
    ///   contribution. `rng` is intentionally unused (no re-deal).
    ///
    /// `MissingPlayerDealing` (a truncated journal dropped a publicly-acked dealing)
    /// surfaces as `Err` so the caller can sit out the epoch gracefully — never crash.
    pub fn resume(
        namespace: &[u8],
        epoch: u64,
        committee: Set<PeerPubkey>,
        me_key: Ed25519PrivateKey,
        records: Vec<JournalRecord>,
    ) -> Result<Resumed, DkgError> {
        let me = me_key.public_key();
        let info = info_for(namespace, epoch, committee)?;

        let mut log_map: BTreeMap<PeerPubkey, DealerLog<MinSig, PeerPubkey>> = BTreeMap::new();
        let mut signed_logs: BTreeMap<PeerPubkey, DealerReveal> = BTreeMap::new();
        let mut own_seal = false;
        // (dealer, pub, priv) dealings to feed `Player::resume` (incl. our own
        // self-dealing — it rebuilds `view[me]`). Player-only, so the self-dealing
        // is ALWAYS re-fed; there is no re-deal branch to exclude it from.
        let mut received: Vec<(PeerPubkey, DealerCommitment, DealerPrivMsg)> = Vec::new();
        for record in records {
            match record {
                JournalRecord::ReceivedDealing(dealer, pub_msg, priv_msg) => {
                    received.push((dealer, *pub_msg, *priv_msg));
                }
                // Mark `sealed` only AFTER `check` succeeds and populates
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
            }
        }

        // Rebuild `Player.view` and capture the regenerated per-dealer acks
        // (`Player::resume` re-emits one ack for every dealing it replays). These
        // MUST be re-broadcast (below) or peers can't seal the logs we'd later fetch.
        let (player, acks) =
            Player::resume::<N3f1>(info.clone(), me_key.clone(), &log_map, received)?;

        let mut logs = Logs::<MinSig, PeerPubkey, N3f1>::new(info.clone());
        let mut recorded = BTreeSet::new();
        for (pk, log) in log_map {
            recorded.insert(pk.clone());
            logs.record(pk, log);
        }

        // Re-emit a point-to-point Ack to every PEER dealer whose dealing we
        // replayed, so a peer that lost our prior ack can still seal its log. The
        // self-ack is dropped — we retired our own dealer (player-only), so there is
        // no local dealer to consume it and broadcasting an ack to ourselves is a
        // no-op (the seal that consumed it already happened pre-crash, captured in our
        // journaled `OwnSeal`).
        let mut outgoing: Vec<Outgoing> = acks
            .into_iter()
            .filter(|(dealer, _)| *dealer != me)
            .map(|(dealer, ack)| Outgoing {
                target: Target::Direct(dealer),
                msg: DkgMsg {
                    ceremony_epoch: epoch,
                    body: DkgBody::Ack(ack),
                },
            })
            .collect();

        // If we sealed before the crash, re-broadcast our journaled log VERBATIM so
        // any peer that missed it records OURS — no re-deal. (No own-seal → we are
        // simply an absent dealer; the n−f survivors finalize and our share is
        // recovered as a player. The dealer role is retired either way.)
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

        Ok(Resumed {
            ceremony: Self {
                epoch,
                info,
                dealer: None,
                player: Some(player),
                logs,
                pending_pub: BTreeMap::new(),
                pending_priv: BTreeMap::new(),
                recorded,
                signed_logs,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::outcome::group_public_key;
    use commonware_codec::Encode as _;
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
            let (cer, step) =
                DkgCeremony::start(&mut rng, ns, 0, committee.clone(), k.clone()).expect("start");
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

    /// Drive a fresh 4-party ceremony to the point where node-0 has acked every
    /// dealer and SEALED, capturing node-0's journal records. Returns
    /// `(committee, key0, journal0)` — the inputs a resume test reconstructs node-0
    /// from.
    fn run_to_node0_sealed(seed: u64) -> (Set<PeerPubkey>, Ed25519PrivateKey, Vec<JournalRecord>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<Ed25519PrivateKey> =
            (0..4).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let committee = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let ns = b"FLUENT_DPOS_V1_test";

        let mut ceremonies: BTreeMap<PeerPubkey, DkgCeremony> = BTreeMap::new();
        let mut queue: Vec<(PeerPubkey, Outgoing)> = Vec::new();
        let pk0 = keys[0].public_key();
        let mut journal0: Vec<JournalRecord> = Vec::new();

        for k in &keys {
            let (cer, step) =
                DkgCeremony::start(&mut rng, ns, 0, committee.clone(), k.clone()).expect("start");
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
                    let recipients: Vec<PeerPubkey> =
                        ceremonies.keys().filter(|pk| **pk != from).cloned().collect();
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
        let mut live = DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee2, key0b, journal0b)
            .expect("live resume");
        assert!(
            live.ceremony.own_log_recorded(&me0),
            "own-seal record ⇒ our own log is recorded (no re-deal)"
        );
        assert!(
            live.ceremony.dealing_closed(),
            "player-only restore retires the dealer role"
        );
        let (out_live, share_live) = live.ceremony.finalize(&mut rng_live).expect("finalize live");

        // Resumed node-0: rebuild from the journal alone (a DIFFERENT rng seed for
        // the re-spawn would, under a re-deal, diverge — player-only restore makes it
        // irrelevant: no rng is consumed).
        let mut rng_res = StdRng::seed_from_u64(99);
        let mut resumed = DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, journal0)
            .expect("resume");
        assert!(resumed.ceremony.own_log_recorded(&me0));
        let (out_res, share_res) = resumed.ceremony.finalize(&mut rng_res).expect("finalize resumed");

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

        match DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, journal0) {
            Err(DkgError::MissingPlayerDealing) => {}
            Err(other) => panic!("expected MissingPlayerDealing, got {other:?}"),
            Ok(_) => panic!("a missing acked dealing must fail resume"),
        }
    }

    /// No own-seal in the journal (crashed before seal) ⇒ player-only restore RETIRES
    /// the dealer role (never re-deals): no live dealer, not sealed, and the only
    /// outgoing messages are the re-emitted peer acks (no fresh commitment/shares).
    /// The node sits out dealing and recovers its share from the survivors' logs.
    #[test]
    fn resume_before_seal_is_absentee_not_redeal() {
        // A journal with ONLY peer logs + received dealings, no OwnSeal: take a sealed
        // run and strip our OwnSeal + own self-deal (as if we crashed pre-seal).
        let (committee, key0, journal0) = run_to_node0_sealed(13);
        let me0 = key0.public_key();
        let pre_seal: Vec<JournalRecord> = journal0
            .into_iter()
            .filter(|r| {
                !matches!(r, JournalRecord::OwnSeal(_))
                    && !matches!(r, JournalRecord::ReceivedDealing(d, _, _) if *d == me0)
            })
            .collect();

        let resumed = DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, pre_seal)
            .expect("resume pre-seal");
        assert!(
            !resumed.ceremony.own_log_recorded(&me0),
            "no OwnSeal ⇒ our own log is NOT recorded (we sit out, recover as a player)"
        );
        assert!(
            resumed.ceremony.dealing_closed(),
            "player-only restore NEVER keeps a live dealer (no re-deal)"
        );
        assert!(
            !resumed
                .outgoing
                .iter()
                .any(|o| matches!(o.msg.body, DkgBody::Commitment(_) | DkgBody::Share(_))),
            "a pre-seal resume must NOT re-deal a fresh commitment/share"
        );
        assert!(
            resumed
                .outgoing
                .iter()
                .all(|o| matches!(o.msg.body, DkgBody::Ack(_))),
            "the only outgoing on a pre-seal resume are the re-emitted peer acks"
        );
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

        let resumed = DkgCeremony::resume(b"FLUENT_DPOS_V1_test", 0, committee, key0, journal0)
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
