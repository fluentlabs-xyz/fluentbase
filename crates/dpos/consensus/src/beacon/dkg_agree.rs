//! The value `committee[E+1]` agrees over p2p during epoch `E` — and, once
//! certified by the agreement instance, the body of the artifact served to
//! peers.
//!
//! [`DkgProposal::logs`] is the pinned dealer-log set: `(dealer idx,
//! keccak256(SignedDealerLog))` over `committee[target_epoch]`, the same value
//! `OrderBlock.dkg_logs` used to carry. Its canonicalisation rules are copied
//! deliberately from that field and enforced HERE, at decode, so the proposal
//! digest is a function of the SET and not of a proposer's ordering — a
//! permuted or duplicated encoding of one set would otherwise be a second
//! payload the instance could split on.
//!
//! [`DkgProposal::group_key`] is `PK_{target_epoch}` and travels as a
//! mandatory fail-fast cross-check, never as the subject of agreement: it is a
//! deterministic function of `logs` (`Σ commitments` over the canonical
//! selection — no rng, no interpolation), so every voter recomputes it and a
//! silent drift in that determinism surfaces here as a refused proposal rather
//! than later as an undiagnosed epoch death on the ordering plane. It is
//! encoded with the [`crate::beacon::outcome`] codec, the same one that
//! serialises the block field and the durable share file.
//!
//! [`DkgProposal::confirms`] carries the share-confirmation count that
//! [`entry_bar`] reads — the number of members that say they can finalize over
//! the pinned set, which is the number that decides the epoch's fate and had no
//! protocol representation at all before it. The confirmations live in the
//! PAYLOAD rather than in local receive state so the acceptance predicate stays a
//! pure function of the proposal: a bar counted from what a node happens to have
//! received would make honest nodes diverge, which is the failure this whole plane
//! exists to remove. [`ConfirmPool`] is the collection side — the beacon actor
//! signs into it, `propose` draws the covering set out of it, and `verify` takes
//! nothing from it but the namespace.
//!
//! On top of the value sits the agreement instance's application half:
//! [`DkgAgree`] (`Automaton` + `Relay`) and [`DkgReporter`]. The one rule that
//! governs all of it: `verify` returns `false` ONLY for a proposal that is
//! permanently unacceptable to every honest node, and PARKS for everything else.
//! `verify → false` is `TimeoutReason::InvalidProposal`, i.e. an immediate
//! nullify, so expressing "I do not hold that yet" as `false` would turn every
//! delivery race into a lost view.
//!
//! The instance agrees ONE value and [`certified_value`] is what makes that
//! structural: once a value is certified, every later view re-proposes it and
//! `verify` refuses anything else. Without that bar simplex's per-view rule alone
//! would let a quorum that missed a body at view `v` certify a different set at
//! `v+1`, leaving two artifacts that both verify against `committee[target_epoch]`.

use alloy_primitives::{keccak256, B256};
use bytes::{Buf, BufMut};
use commonware_broadcast::Broadcaster as _;
use commonware_codec::{Encode as _, EncodeSize, Error, FixedSize, Read, ReadExt as _, Write};
use commonware_consensus::{
    simplex::{
        types::{Activity, Context as SimplexContext, Finalization},
        Plan,
    },
    types::{Epoch, Round, View},
    Automaton, CertifiableAutomaton, Relay, Reporter,
};
use commonware_cryptography::{
    ed25519::{PrivateKey as Ed25519PrivateKey, Signature},
    Committable, Digestible, Signer as _, Verifier as _,
};
use commonware_p2p::Recipients;
use commonware_resolver::Resolver;
use commonware_runtime::{Clock, Metrics, Spawner};
use commonware_utils::{
    channel::{fallible::OneshotExt as _, oneshot},
    vec::NonEmptyVec,
    Faults as _, N3f1,
};
use fluentbase_bls::{PeerPubkey, Scheme as BlsScheme};
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::{
    beacon::{
        actor::DkgLogIndex,
        dkg_transport::BodyMailbox,
        log_resolver::DkgLogKey,
        metrics::BeaconMetrics,
        outcome::{
            encode_outcome, parse_outcome, DkgOutcome, OutcomeError, MAX_BEACON_OUTCOME_SIZE,
        },
    },
    digest::Digest,
};

/// Entry count cap for any committee-indexed set here (`logs`, `recorded`,
/// `confirms`): one entry per seat.
const MAX_SET_LEN: usize = MAX_COMMITTEE_SIZE as usize;

/// `u32` count/length prefix.
const LEN_PREFIX: usize = u32::SIZE;

/// One `(dealer idx, log hash)` entry on the wire.
const LOG_ENTRY_SIZE: usize = u8::SIZE + 32;

/// One member's statement that it holds body-checked dealer logs — the count of
/// these is the number of members that can actually finalize, which is the
/// number that governs the epoch's fate and has no other representation in the
/// protocol.
///
/// The signed bytes bind `target_epoch` AND a digest of `recorded`. Both are
/// load-bearing: a confirmation is lifted out of the DKG envelope into this
/// payload, and the envelope's `ceremony_epoch` is plain framing that does not
/// travel with it, so without the epoch a genuine confirmation replays into
/// every other epoch, and without the set digest a valid signature re-attaches
/// to an inflated `recorded`. `recorded` itself travels in the clear because a
/// verifier must both re-derive that digest and test the superset relation
/// against the proposal's `logs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShareConfirm {
    /// The confirming member's index in `committee[target_epoch]`.
    pub idx: u8,
    /// Part of the signed bytes.
    pub target_epoch: u64,
    /// The body-checked dealer logs this member holds. CANONICAL: strictly
    /// ascending by `idx`, no duplicates, `idx < MAX_COMMITTEE_SIZE`.
    pub recorded: Vec<(u8, B256)>,
    /// Over `target_epoch ‖ keccak256(canonical(recorded))`, under the member's
    /// consensus key.
    pub sig: Signature,
}

/// Domain-separator suffix for share-confirmation signatures, appended to the
/// beacon plane's own namespace by [`ConfirmPool::new`].
///
/// The same ed25519 key signs dealer logs and player acks inside the ceremony, so
/// the confirmation needs a domain of its own. A signed message is
/// `union_unique(namespace, msg)`, which length-prefixes the namespace, so a
/// distinct suffix is the whole requirement — no prefix-freedom against the base
/// is needed or claimed.
const CONFIRM_SUFFIX: &[u8] = b"_DKG_CONFIRM";

/// The bytes a [`ShareConfirm`] signs: `target_epoch ‖ keccak256(canonical(recorded))`.
///
/// Both halves are load-bearing and neither is recoverable from the framing. A
/// confirmation is lifted out of the [`crate::beacon::dkg_msg::DkgMsg`] envelope
/// into a proposal payload, and that envelope's `ceremony_epoch` is unsigned
/// framing that does not travel with it — so without the epoch a genuine
/// confirmation for `E` replays inside a proposal for any `E'` whose `logs` the
/// confirmed set happens to cover, clearing the entry bar before that many members
/// have confirmed anything for `E'`. Without the set digest a valid signature
/// re-attaches to an inflated `recorded`, which buys the same inflation inside one
/// epoch. The set is digested rather than signed whole so the signed message stays
/// fixed-size.
fn confirm_message(target_epoch: u64, recorded: &[(u8, B256)]) -> [u8; 40] {
    let mut canonical = Vec::with_capacity(log_set_size(recorded));
    write_log_set(recorded, &mut canonical);
    let mut msg = [0u8; 40];
    msg[..8].copy_from_slice(&target_epoch.to_be_bytes());
    msg[8..].copy_from_slice(keccak256(canonical).as_slice());
    msg
}

impl ShareConfirm {
    /// Sign this node's confirmation for `target_epoch`. `recorded` must already be
    /// canonical — strictly ascending by `idx`, no duplicates — or the digest names
    /// a set no decoder will ever reproduce.
    pub fn sign(
        namespace: &[u8],
        signer: &Ed25519PrivateKey,
        idx: u8,
        target_epoch: u64,
        recorded: Vec<(u8, B256)>,
    ) -> Self {
        let sig = signer.sign(namespace, &confirm_message(target_epoch, &recorded));
        Self {
            idx,
            target_epoch,
            recorded,
            sig,
        }
    }

    /// Whether `member` really signed this confirmation of this set for this epoch.
    pub fn verify(&self, namespace: &[u8], member: &PeerPubkey) -> bool {
        member.verify(
            namespace,
            &confirm_message(self.target_epoch, &self.recorded),
            &self.sig,
        )
    }

    /// Whether this confirmation COVERS `logs`: the confirmed set contains every
    /// pinned entry, hash included.
    ///
    /// Hash-sensitive on purpose. A member that recorded a DIFFERENT body at a
    /// pinned seat cannot finalize over that seat, so counting it towards the bar
    /// would count a member that the epoch's fate does not include. Both sides are
    /// strictly ascending by `idx` (enforced at decode), so this is one merge walk.
    pub fn covers(&self, logs: &[(u8, B256)]) -> bool {
        let mut recorded = self.recorded.iter().peekable();
        for entry in logs {
            loop {
                match recorded.peek() {
                    Some(held) if held.0 < entry.0 => {
                        recorded.next();
                    }
                    Some(held) if *held == entry => {
                        recorded.next();
                        break;
                    }
                    _ => return false,
                }
            }
        }
        true
    }
}

/// Whether `confirm` is a confirmation this instance may count: it names this
/// target epoch, it names a seat that exists, and the member sitting there signed
/// it.
///
/// Every clause is a permanent property of the confirmation under
/// `committee[target_epoch]`, so two honest nodes always agree on it — which is
/// what lets a proposal be REJECTED on it rather than parked. It deliberately does
/// not check that the confirmed hashes are real: a member's confirmation is its own
/// claim, and a Byzantine member can always overstate what it holds. That inflates
/// the count by at most `f`, which the fault model already pays for.
fn confirm_is_countable(
    confirm: &ShareConfirm,
    namespace: &[u8],
    committee: &[PeerPubkey],
    target_epoch: u64,
) -> bool {
    confirm.target_epoch == target_epoch
        && committee
            .get(confirm.idx as usize)
            .is_some_and(|member| confirm.verify(namespace, member))
}

/// The agreement view from which the margin is released and the bar is the bare
/// quorum.
///
/// Deliberately SMALL. No recorded incident would have been prevented by the margin
/// at all — every one landed at zero share-holders, not at "quorum minus one" — so
/// the margin refuses to ENTER an epoch that stands one fault from a stop and
/// repairs nothing. A prophylactic that nothing has ever needed is worth two views
/// of boundary latency and no more; raising it buys nothing measurable and costs
/// that latency directly.
///
/// It is a VIEW count and never a duration. The view number is agreed by the
/// protocol itself — it rides every vote and the finalization certificate — so
/// every honest node in a view computes the same bar. A node-local timer would be
/// the liveness split the fixed rule exists to prevent: two nodes would drop the
/// margin at different moments and disagree on whether a proposal carrying only
/// quorum confirmations is acceptable, and the stricter one would nullify a view
/// the other certified.
pub(crate) const MARGIN_RELEASE_VIEW: u64 = 3;

/// The margin above the quorum, as a function of the fault bound alone.
///
/// `m <= f` is the hard constraint: the bar is `quorum(n) + m = n - f + m`, so any
/// larger `m` would be unsatisfiable even with every member present. At `f = 1` it
/// is 0, which is why committees of 4-6 — the dpos smoke, the production path and
/// the growth case's start state — see no change from it: at those sizes any
/// nonzero margin would mean "every member must succeed", which is stricter, not
/// sturdier.
const fn margin(f: u32) -> u32 {
    if f / 2 < 2 {
        f / 2
    } else {
        2
    }
}

/// How many share-confirmations a proposal must carry to be acceptable at `view`.
///
/// NEVER CONFIGURABLE, and the reason is liveness rather than taste. `PK_E` is a
/// pure function of the pinned set, so a node running a stricter bar that DOES
/// finalize derives the identical key — it simply refuses to finalize, demotes to
/// verify-only and votes false on the change-boundary block. A configurable margin
/// therefore degrades into "the strictest node self-removes", and `> f` strict
/// nodes halt the epoch. Neither `f` nor the margin is a parameter here for exactly
/// that reason: both follow from `n` and the fault model, which the whole network
/// already shares. Adding a knob — or a caller-supplied `f` — is a protocol change
/// and belongs in review, not in a config file.
///
/// Non-increasing in `view`, which is what makes rejecting on it safe: a value
/// certified while the margin held stays acceptable in every later view, so the
/// release can never orphan an agreement already reached.
pub(crate) fn entry_bar(n: usize, view: View) -> usize {
    if n == 0 {
        return 0;
    }
    let quorum = N3f1::quorum(n) as usize;
    if view.get() >= MARGIN_RELEASE_VIEW {
        quorum
    } else {
        quorum + margin(N3f1::max_faults(n)) as usize
    }
}

/// The share-confirmations this node has collected per target epoch, and the
/// namespace they are signed under.
///
/// ONE source for that namespace, deliberately. The beacon actor signs with it and
/// the acceptance predicate verifies with it, and a mismatch between two
/// independently derived copies would be invisible: every confirmation would simply
/// fail to verify and the bar would never be met, on a plane whose entire failure
/// mode is already "nothing happens". The pool is constructed once and handed to
/// both.
///
/// The MAP feeds `propose` only. `verify` reads the namespace out of it and nothing
/// else: the confirmations a proposal is judged on travel inside that proposal,
/// never from here. A predicate that counted local receive state would stop being a
/// pure function of the proposal, and honest nodes would diverge on delivery order —
/// the exact failure this plane exists to remove.
#[derive(Clone)]
pub struct ConfirmPool {
    namespace: Arc<Vec<u8>>,
    confirms: Arc<Mutex<BTreeMap<u64, BTreeMap<u8, ShareConfirm>>>>,
    /// Bumped whenever EITHER of `build_proposal`'s two node-local inputs grows —
    /// this map, or the beacon actor's recorded dealer-log index. See
    /// [`ConfirmPool::subscribe`].
    inputs: Arc<watch::Sender<u64>>,
}

impl ConfirmPool {
    /// A pool whose confirmations are signed under `base_namespace ‖ "_DKG_CONFIRM"`.
    pub fn new(base_namespace: &[u8]) -> Self {
        let mut namespace = Vec::with_capacity(base_namespace.len() + CONFIRM_SUFFIX.len());
        namespace.extend_from_slice(base_namespace);
        namespace.extend_from_slice(CONFIRM_SUFFIX);
        Self {
            namespace: Arc::new(namespace),
            confirms: Arc::new(Mutex::new(BTreeMap::new())),
            inputs: Arc::new(watch::channel(0).0),
        }
    }

    pub fn namespace(&self) -> &[u8] {
        &self.namespace
    }

    /// A subscription that fires when a leader's refusal could have become stale.
    ///
    /// `build_proposal` reads two node-local inputs — this pool and the beacon
    /// actor's recorded dealer-log index — and a leader that cannot build yet has
    /// to be woken by whichever of them grows. Both edges land here rather than one
    /// each: the index is a bare `Arc<RwLock<_>>` shared with the plane, so giving
    /// it a channel of its own would mean two subscriptions and two wakeups for one
    /// predicate. The actor bumps it from [`ConfirmPool::record`] and from
    /// `publish_recorded_logs`.
    ///
    /// `watch` and not `Notify`: the subscription marks the current value seen at
    /// subscribe time, so a growth landing between the subscribe and the failed
    /// build is still delivered. `Notify::notify_waiters` would drop it, and the
    /// leader would then sleep out its whole view holding an answer it already had.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.inputs.subscribe()
    }

    /// Wake every subscriber. Called by the beacon actor when the recorded
    /// dealer-log index grows; [`ConfirmPool::record`] does it for this map.
    pub(crate) fn note_inputs_grew(&self) {
        self.inputs.send_modify(|seq| *seq = seq.wrapping_add(1));
    }

    /// Verify `confirm` against `committee[confirm.target_epoch]` and keep it if it
    /// says more than what is already held for that seat. Returns whether it was
    /// stored.
    ///
    /// Widest-wins rather than latest-wins: a member's body-checked set only ever
    /// grows (`record_checked_log` is first-wins and irreversible), so the widest
    /// confirmation it ever signed is its truest one, and a replayed older
    /// confirmation cannot narrow what this node counts.
    pub(crate) fn record(&self, committee: &[PeerPubkey], confirm: ShareConfirm) -> bool {
        if !confirm_is_countable(&confirm, self.namespace(), committee, confirm.target_epoch) {
            return false;
        }
        let mut pool = self.lock();
        let stored = match pool
            .entry(confirm.target_epoch)
            .or_default()
            .entry(confirm.idx)
        {
            std::collections::btree_map::Entry::Occupied(mut held) => {
                if confirm.recorded.len() > held.get().recorded.len() {
                    held.insert(confirm);
                    true
                } else {
                    false
                }
            }
            std::collections::btree_map::Entry::Vacant(seat) => {
                seat.insert(confirm);
                true
            }
        };
        if stored {
            drop(pool);
            self.note_inputs_grew();
        }
        stored
    }

    /// The confirmations for `epoch` that cover `logs`, ascending by member index —
    /// the canonical order [`DkgProposal`]'s codec requires.
    pub(crate) fn covering(&self, epoch: u64, logs: &[(u8, B256)]) -> Vec<ShareConfirm> {
        self.lock()
            .get(&epoch)
            .into_iter()
            .flat_map(|seats| seats.values())
            .filter(|confirm| confirm.covers(logs))
            .cloned()
            .collect()
    }

    /// Drop every epoch the predicate rejects. Called from the beacon actor's
    /// height sweep on the same predicate that retains the ceremonies themselves.
    pub fn retain(&self, keep: impl Fn(u64) -> bool) {
        self.lock().retain(|epoch, _| keep(*epoch));
    }

    /// A poisoned lock recovers rather than propagating: this map only ever raises a
    /// count that a refusal is already the safe answer to, so reading it through a
    /// panic cannot produce an unsafe verdict, while panicking here would take the
    /// beacon actor down over a bookkeeping map.
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, BTreeMap<u8, ShareConfirm>>> {
        self.confirms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// What `committee[target_epoch]` agrees over p2p, and — once certified — the
/// artifact body served to peers. See the module doc for what each field is and
/// why it is here.
///
/// CANONICAL: `logs` and every `recorded` strictly ascending by `idx`;
/// `confirms` strictly ascending by member index; no duplicates; every index
/// `< MAX_COMMITTEE_SIZE`; every set at most `MAX_COMMITTEE_SIZE` long.
/// Enforced at decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DkgProposal {
    pub target_epoch: u64,
    pub logs: Vec<(u8, B256)>,
    pub group_key: DkgOutcome,
    pub(crate) confirms: Vec<ShareConfirm>,
}

impl DkgProposal {
    /// keccak256 over the canonical encoding — the payload identity the
    /// agreement instance votes on.
    pub fn digest(&self) -> Digest {
        Digest(keccak256(self.encode()))
    }
}

fn write_log_set(set: &[(u8, B256)], buf: &mut impl BufMut) {
    (set.len() as u32).write(buf);
    for (idx, hash) in set {
        idx.write(buf);
        buf.put_slice(hash.as_slice());
    }
}

fn log_set_size(set: &[(u8, B256)]) -> usize {
    LEN_PREFIX + set.len() * LOG_ENTRY_SIZE
}

/// Decode a `(idx, hash)` set under the canonical rules.
///
/// The COUNT bound does not bound the VALUE: `idx` is a `u8`, so without the
/// per-entry check an entry could name seat 200 of a 51-seat committee and wedge
/// whatever consumes it. `MAX_COMMITTEE_SIZE` is a network-wide constant, so
/// this rejects identically on every node — a bound against the live committee
/// length would instead make decoding depend on node-local state.
fn read_log_set(buf: &mut impl Buf) -> Result<Vec<(u8, B256)>, Error> {
    let count = u32::read(buf)? as usize;
    if count > MAX_SET_LEN {
        return Err(Error::Invalid(
            "dkg_agree",
            "log set exceeds MAX_COMMITTEE_SIZE entries",
        ));
    }
    if count * LOG_ENTRY_SIZE > buf.remaining() {
        return Err(Error::EndOfBuffer);
    }
    let mut set = Vec::with_capacity(count);
    let mut prev: Option<u8> = None;
    for _ in 0..count {
        let idx = u8::read(buf)?;
        if idx as usize >= MAX_SET_LEN {
            return Err(Error::Invalid(
                "dkg_agree",
                "log set idx exceeds MAX_COMMITTEE_SIZE",
            ));
        }
        if prev.is_some_and(|p| idx <= p) {
            return Err(Error::Invalid(
                "dkg_agree",
                "log set idx not strictly ascending",
            ));
        }
        prev = Some(idx);
        set.push((idx, B256::from(<[u8; 32]>::read(buf)?)));
    }
    Ok(set)
}

// Wire: idx(1) ‖ target_epoch(8) ‖ recorded_count(4) + count*(idx(1) ‖ hash(32))
//   ‖ sig(64).
impl Write for ShareConfirm {
    fn write(&self, buf: &mut impl BufMut) {
        self.idx.write(buf);
        self.target_epoch.write(buf);
        write_log_set(&self.recorded, buf);
        self.sig.write(buf);
    }
}

impl EncodeSize for ShareConfirm {
    fn encode_size(&self) -> usize {
        u8::SIZE + self.target_epoch.encode_size() + log_set_size(&self.recorded) + Signature::SIZE
    }
}

impl Read for ShareConfirm {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &Self::Cfg) -> Result<Self, Error> {
        let idx = u8::read(buf)?;
        if idx as usize >= MAX_SET_LEN {
            return Err(Error::Invalid(
                "dkg_agree",
                "confirm idx exceeds MAX_COMMITTEE_SIZE",
            ));
        }
        let target_epoch = u64::read(buf)?;
        let recorded = read_log_set(buf)?;
        let sig = Signature::read(buf)?;
        Ok(Self {
            idx,
            target_epoch,
            recorded,
            sig,
        })
    }
}

// Wire: target_epoch(8) ‖ logs_count(4) + count*(idx(1) ‖ hash(32))
//   ‖ group_key_len(4)+bytes ‖ confirms_count(4) + count*ShareConfirm.
// `group_key` is length-prefixed because the commonware `Output` decoder needs
// the committee-size config this layer supplies through `parse_outcome`.
impl Write for DkgProposal {
    fn write(&self, buf: &mut impl BufMut) {
        self.target_epoch.write(buf);
        write_log_set(&self.logs, buf);
        let key = encode_outcome(&self.group_key);
        (key.len() as u32).write(buf);
        buf.put_slice(&key);
        (self.confirms.len() as u32).write(buf);
        for confirm in &self.confirms {
            confirm.write(buf);
        }
    }
}

impl EncodeSize for DkgProposal {
    fn encode_size(&self) -> usize {
        self.target_epoch.encode_size()
            + log_set_size(&self.logs)
            + LEN_PREFIX
            + self.group_key.encode_size()
            + LEN_PREFIX
            + self.confirms.iter().map(|c| c.encode_size()).sum::<usize>()
    }
}

impl Read for DkgProposal {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &Self::Cfg) -> Result<Self, Error> {
        let target_epoch = u64::read(buf)?;
        let logs = read_log_set(buf)?;

        let key_len = u32::read(buf)? as usize;
        if key_len > MAX_BEACON_OUTCOME_SIZE {
            return Err(Error::Invalid(
                "dkg_agree",
                "group_key exceeds MAX_BEACON_OUTCOME_SIZE",
            ));
        }
        if key_len > buf.remaining() {
            return Err(Error::EndOfBuffer);
        }
        let key_bytes = buf.copy_to_bytes(key_len);
        let group_key = parse_outcome(&key_bytes).map_err(|e| match e {
            OutcomeError::Decode(err) => err,
            OutcomeError::TrailingBytes => {
                Error::Invalid("dkg_agree", "group_key has trailing bytes")
            }
        })?;

        let count = u32::read(buf)? as usize;
        if count > MAX_SET_LEN {
            return Err(Error::Invalid(
                "dkg_agree",
                "confirms exceeds MAX_COMMITTEE_SIZE entries",
            ));
        }
        let mut confirms = Vec::with_capacity(count);
        let mut prev: Option<u8> = None;
        for _ in 0..count {
            let confirm = ShareConfirm::read(buf)?;
            if prev.is_some_and(|p| confirm.idx <= p) {
                return Err(Error::Invalid(
                    "dkg_agree",
                    "confirms idx not strictly ascending",
                ));
            }
            prev = Some(confirm.idx);
            confirms.push(confirm);
        }

        Ok(Self {
            target_epoch,
            logs,
            group_key,
            confirms,
        })
    }
}

impl Committable for DkgProposal {
    type Commitment = Digest;

    fn commitment(&self) -> Self::Commitment {
        self.digest()
    }
}

impl Digestible for DkgProposal {
    type Digest = Digest;

    fn digest(&self) -> Self::Digest {
        self.digest()
    }
}

/// The agreement's output: the certified payload plus simplex's own finalization
/// certificate, which verifies standalone against `committee[target_epoch]`.
pub type AgreedArtifact = (DkgProposal, Finalization<BlsScheme, Digest>);

/// What this node can say about a candidate pinned dealer-log set.
///
/// The three non-`Derived` arms are NOT interchangeable, and the split is the
/// whole point: only [`Unusable`](PinnedDerive::Unusable) is a property of the
/// proposal, so only it may become a `verify` verdict. The other two are
/// properties of THIS node's delivery state, and a verdict drawn from those
/// would nullify a view that a proposal-identical honest node accepts.
#[derive(Clone, Debug)]
pub(crate) enum PinnedDerive {
    /// Every named body is held under the named hash, and the public half of the
    /// ceremony over exactly those dealers is this.
    Derived(Box<DkgOutcome>),
    /// The named bodies at these committee indices are not held, or are held
    /// under a different hash. The caller aims a targeted fetch at them and
    /// parks; a body that arrives later makes the same proposal acceptable.
    Missing(Vec<u8>),
    /// The ceremony state needed to answer is not available here at all — no
    /// live ceremony for the epoch, an unreadable roster. Park.
    Unavailable,
    /// Every named body is held, and no group key follows from them. This is a
    /// pure function of the proposal (the selection is canonical and the sum
    /// takes no node-local input), so every honest node reaches it, which is
    /// what makes it safe to vote on.
    Unusable,
}

/// The seam between the agreement automaton and the ceremony state that holds
/// the dealer-log BODIES.
///
/// The automaton is handed HASHES ([`DkgLogIndex`]) and could compare those on
/// its own, but the `group_key` clause of the acceptance predicate is
/// `Σ commitments` over the selected dealers, and a commitment lives in a body.
/// Bodies are owned single-threaded by [`crate::beacon::actor::DkgActor`], so the
/// answer crosses a channel and the call is async — the same shape
/// [`crate::beacon::log_resolver::LogHandler`] uses for `Produce`/`Deliver`.
///
/// IMPLEMENTOR CONTRACT: return [`PinnedDerive::Unusable`] ONLY when every named
/// body is held. Any local inability to answer — no ceremony, no roster, a lock
/// this implementation declined to take — is [`PinnedDerive::Unavailable`].
/// Getting that wrong converts a node-local gap into a nullified view, which is
/// exactly the failure the acceptance predicate is written to avoid.
pub(crate) trait PinnedLogs: Clone + Send + 'static {
    fn derive(&self, pinned: BTreeMap<u8, B256>) -> impl Future<Output = PinnedDerive> + Send;
}

/// The body `propose` built for the relay to broadcast, and the round it built it
/// for.
///
/// ROUND-KEYED, and it has to be: a propose task can outlive its own view. Since
/// [`DkgAgree::build_proposal`] waits inside the leader's window and resolves
/// whenever an input grows, a task asked at round `r` can still be inside
/// `derive` when the voter has moved on and a second task has already armed the
/// slot for a later round `r'`. On an unkeyed slot that late write replaces `r'`'s
/// body: the relay then finds a body whose digest is not the one consensus asked
/// it to broadcast, refuses to send it, and `r'` burns its whole certification
/// timeout while its peers park in `verify` on a body that never left.
///
/// So a write only ever moves the round forward, and the round OUTLIVES the body:
/// the relay takes the proposal but leaves the round behind, because a late task
/// would otherwise just re-arm the emptied slot with its stale body.
#[derive(Clone, Default)]
struct BuiltProposal(Arc<Mutex<Option<Armed>>>);

struct Armed {
    round: Round,
    /// `None` once the relay has taken the body; the round stays.
    proposal: Option<DkgProposal>,
}

impl BuiltProposal {
    /// Offer `proposal` as the body for `round`. Returns whether the relay will
    /// find it — `false` means a later round owns the slot and this proposal is
    /// stale, or the lock is poisoned.
    fn arm(&self, round: Round, proposal: DkgProposal) -> bool {
        let Ok(mut slot) = self.0.lock() else {
            warn!(
                epoch = round.epoch().get(),
                view = round.view().get(),
                "dkg agree: the built-proposal slot is poisoned, not proposing"
            );
            return false;
        };
        if slot.as_ref().is_some_and(|held| held.round > round) {
            debug!(
                epoch = round.epoch().get(),
                view = round.view().get(),
                "dkg agree: built a proposal for a round the instance has left, discarding it"
            );
            return false;
        }
        *slot = Some(Armed {
            round,
            proposal: Some(proposal),
        });
        true
    }

    /// The body the relay broadcasts, once.
    fn take(&self) -> Option<DkgProposal> {
        self.0
            .lock()
            .ok()
            .and_then(|mut slot| slot.as_mut().and_then(|held| held.proposal.take()))
    }
}

/// The `Automaton`/`Relay` half of the epoch-key agreement instance.
///
/// One instance per target epoch, and it agrees exactly one value: the pinned
/// dealer-log set for `committee[target_epoch]`. There is no parent chain — the
/// genesis digest is a constant of the target epoch and every proposal builds on
/// it.
pub(crate) struct DkgAgree<E, R, L> {
    context: E,
    target_epoch: u64,
    /// `committee[target_epoch]` in consensus order: the index space `logs` and
    /// `confirms` are numbered against, and the dealer identities the resolver
    /// is keyed by.
    committee: Vec<PeerPubkey>,
    bodies: BodyMailbox,
    /// The existing `{epoch, dealer}` dealer-log resolver.
    logs: R,
    /// `epoch → idx → keccak256(SignedDealerLog)` for the logs this node has
    /// recorded with the body checked. Read to build a proposal; never read to
    /// decide one.
    recorded: DkgLogIndex,
    pinned: L,
    /// The share-confirmations this node has collected, and the namespace they are
    /// signed under. `propose` draws the covering set from it; `verify` takes only
    /// the namespace.
    confirms: ConfirmPool,
    /// Set by `propose`, taken by `Relay::broadcast(Plan::Propose)`. Shared
    /// because the voter holds independent clones for the two roles — mirrors
    /// `Inline`'s `last_built`.
    last_built: BuiltProposal,
    notes: AgreementNotes,
}

impl<E: Clone, R: Clone, L: Clone> Clone for DkgAgree<E, R, L> {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            target_epoch: self.target_epoch,
            committee: self.committee.clone(),
            bodies: self.bodies.clone(),
            logs: self.logs.clone(),
            recorded: self.recorded.clone(),
            pinned: self.pinned.clone(),
            confirms: self.confirms.clone(),
            last_built: self.last_built.clone(),
            notes: self.notes.clone(),
        }
    }
}

/// Everything one agreement instance's application half needs beyond its runtime
/// context. A struct rather than a parameter list because the members are all
/// distinct handles into the beacon plane and a positional call site invites a
/// silent swap between two of them.
pub(crate) struct DkgAgreeConfig<R, L> {
    pub target_epoch: u64,
    pub committee: Vec<PeerPubkey>,
    pub bodies: BodyMailbox,
    pub logs: R,
    pub recorded: DkgLogIndex,
    pub pinned: L,
    /// PRECONDITION: the same pool the beacon actor signs into. Its namespace is
    /// what confirmations are verified under here, so a second pool built from a
    /// different base namespace would reject every honest confirmation and the
    /// entry bar would never be met — silently.
    pub confirms: ConfirmPool,
    pub metrics: BeaconMetrics,
}

impl<E, R, L> DkgAgree<E, R, L> {
    pub fn new(context: E, cfg: DkgAgreeConfig<R, L>) -> Self {
        Self {
            context,
            target_epoch: cfg.target_epoch,
            committee: cfg.committee,
            bodies: cfg.bodies,
            logs: cfg.logs,
            recorded: cfg.recorded,
            pinned: cfg.pinned,
            confirms: cfg.confirms,
            last_built: BuiltProposal::default(),
            notes: AgreementNotes::new(cfg.metrics),
        }
    }

    /// The `(target epoch, reason)` refusals already warned about — the warn-once
    /// ledger, so a test can assert WHICH arm a refusal took and not only how many
    /// arms fired.
    #[cfg(test)]
    pub(crate) fn reported(&self) -> BTreeSet<(u64, &'static str)> {
        self.notes
            .reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// The genesis digest of the agreement instance for `epoch`.
///
/// A single-height instance has no parent chain, so this is just a domain-tagged
/// constant: every honest member computes the same one, and it differs per epoch
/// so a proposal cannot be replayed into another target's instance.
pub(crate) fn agreement_genesis(epoch: u64) -> Digest {
    let mut preimage = Vec::with_capacity(GENESIS_TAG.len() + 8);
    preimage.extend_from_slice(GENESIS_TAG);
    preimage.extend_from_slice(&epoch.to_be_bytes());
    Digest(keccak256(preimage))
}

const GENESIS_TAG: &[u8] = b"FLUENT_DKG_AGREE_GENESIS";

/// The value this instance has already certified, read off the parent simplex
/// supplies — or `None` while nothing is certified yet.
///
/// This is the structural bar against two agreed values, and it has to be
/// structural: `simplex` forbids conflicting finalizations WITHIN a view, not
/// across them, so without it a quorum that missed the body at view `v` could
/// certify a different set at `v+1` and the epoch would end with two `PK_E`s that
/// both verify against `committee[target_epoch]` — the exact epoch death this
/// plane exists to prevent.
///
/// The parent is authoritative and needs no local bookkeeping: before `verify` is
/// ever called, simplex has checked that the named parent is certified, that it is
/// at or above the last finalization, and that EVERY view between it and the
/// current one is nullified (`CW/consensus/src/simplex/actors/voter/state.rs:705-744`).
/// A view cannot be both certified and nullified under the fault model — an honest
/// node forfeits notarize once it nullifies (`voter/round.rs:327-336`) and the two
/// quorums intersect in an honest node — so a certified value can never be skipped
/// back over. View 0 is the genesis container, so a parent there means the instance
/// has certified nothing.
const fn certified_value(parent: (View, Digest)) -> Option<Digest> {
    if parent.0.get() == 0 {
        None
    } else {
        Some(parent.1)
    }
}

/// What a decision came to — or that it did not come to one.
///
/// The voter's channel carries two answers; this plane needs three. `Park` is the
/// third: not "yes", not "no", but "nothing this node holds decides it yet". It is
/// a VALUE rather than a never-resolving future so that the parking discipline
/// lives in [`drive`] — the one place that owns the wire to the voter — instead of
/// in every branch that has to remember to spell it.
enum Decision<T> {
    Resolve(T),
    Park,
}

/// A `verify` answer.
///
/// Separate from `Decision<bool>` on purpose: the rule this plane turns on is that
/// `false` is reserved for a proposal permanently unacceptable to EVERY honest node
/// (it reaches the voter as `TimeoutReason::InvalidProposal`, an immediate nullify),
/// so a rejection must be an act of naming, never the value a branch falls into. A
/// new arm cannot write `false` here; it has to choose between [`Self::Reject`] and
/// [`Self::Park`], which is the whole point.
enum Verdict {
    /// The proposal checks out: its pinned set derives its own group key.
    Accept,
    /// Reject ONLY a proposal that is permanently unacceptable to EVERY honest
    /// node — a replaced certified value, a structural violation, a set that
    /// derives no key or the wrong one.
    ///
    /// This reaches the voter as `TimeoutReason::InvalidProposal`, an immediate
    /// nullify. **When in doubt, [`Self::Park`].** "I do not hold that yet" is a
    /// delivery race, not a bad proposal, and answering it here costs the view.
    Reject,
    /// Nothing this node holds decides it yet. Costs nothing: the view runs its
    /// own timeout, and a later round proposing the same set is still accepted.
    Park,
}

impl From<Verdict> for Decision<bool> {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Accept => Decision::Resolve(true),
            Verdict::Reject => Decision::Resolve(false),
            Verdict::Park => Decision::Park,
        }
    }
}

/// Resolve a voter request from a `decision` future, or leave it pending.
///
/// The voter awaits the matching `rx` exactly once per `(context, payload)` and
/// never retries. So this driver ends in exactly one of three ways: it sends the
/// definitive verdict, it parks (holding `tx` open until the view moves on), or it
/// exits because the voter already dropped `rx`.
///
/// The park arm must keep `tx` ALIVE. Dropping it unsent reaches the voter as
/// `Err`, which it reads as `TimeoutReason::IgnoredProposal` and turns into an
/// immediate nullify — the same lost view a `false` would have cost. That is the
/// entire reason this function, and not each caller, owns the parking.
async fn drive<T: Send>(mut tx: oneshot::Sender<T>, decision: impl Future<Output = Decision<T>>) {
    tokio::select! {
        _ = tx.closed() => {}
        decided = decision => match decided {
            Decision::Resolve(verdict) => {
                tx.send_lossy(verdict);
            }
            Decision::Park => tx.closed().await,
        },
    }
}

/// This node's own body-checked dealer-log set for `epoch`, scoped to seats that
/// exist in a committee of `n`.
fn local_set(recorded: &DkgLogIndex, epoch: u64, n: usize) -> BTreeMap<u8, B256> {
    recorded
        .read()
        .ok()
        .and_then(|g| g.get(&epoch).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|(idx, _)| (*idx as usize) < n)
        .collect()
}

/// A proposal this node could not put to the instance because too few members have
/// confirmed they hold a usable share. The two arms mean different things to an
/// operator and must never be collapsed.
const BAR_QUORUM_UNMET: &str = "confirm_quorum_not_met";
const BAR_MARGIN_UNMET: &str = "confirm_margin_not_met";

/// Why this node had nothing it could put to the instance, warned where the
/// refusal happens.
///
/// Warn-once per `(target epoch, reason)`, on the pattern the finalize-deferral
/// path already uses (`beacon::actor`'s `deferred_reported` /
/// `dkg_finalize_deferred`). Without the ledger every refusal would re-warn on
/// every view of a plane whose whole symptom is that views keep passing; with it,
/// the view carried in the message says how long the plane took to get stuck.
#[derive(Clone)]
pub(crate) struct AgreementNotes {
    reported: Arc<Mutex<BTreeSet<(u64, &'static str)>>>,
    metrics: BeaconMetrics,
}

impl AgreementNotes {
    fn new(metrics: BeaconMetrics) -> Self {
        Self {
            reported: Arc::new(Mutex::new(BTreeSet::new())),
            metrics,
        }
    }

    /// Whether this `(epoch, reason)` has not been reported yet — and record it.
    fn first_report(&self, epoch: u64, reason: &'static str) -> bool {
        self.reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((epoch, reason))
    }

    /// Report why this node has nothing to propose at `view`.
    fn refuse(&self, epoch: u64, view: View, reason: &'static str) {
        if self.first_report(epoch, reason) {
            warn!(
                epoch,
                view = view.get(),
                reason,
                "dkg agree: nothing to propose for this target epoch"
            );
        }
    }

    /// Report an unmet entry bar, distinguishing the two cases.
    ///
    /// *Quorum met, margin not met* is a prophylactic refusal and not a fault: the
    /// release at [`MARGIN_RELEASE_VIEW`] will start the epoch on its own. *Quorum
    /// not met* is the opposite — the release will not help, because the members are
    /// genuinely absent. An operator acts on the second and waits out the first, so
    /// one message for both would be worse than none.
    fn bar_unmet(&self, epoch: u64, view: View, covering: usize, quorum: usize, bar: usize) {
        let reason = if covering < quorum {
            BAR_QUORUM_UNMET
        } else {
            BAR_MARGIN_UNMET
        };
        if !self.first_report(epoch, reason) {
            return;
        }
        self.metrics.dkg_agree_bar_unmet.inc();
        if reason == BAR_QUORUM_UNMET {
            warn!(
                epoch,
                view = view.get(),
                covering,
                quorum,
                "dkg agree: too few members confirm they hold the pinned dealer logs — the \
                 margin release will not help, they are absent"
            );
        } else {
            warn!(
                epoch,
                view = view.get(),
                covering,
                bar,
                release_view = MARGIN_RELEASE_VIEW,
                "dkg agree: the quorum confirms the pinned dealer logs but the margin does not \
                 — the epoch will start at the release view"
            );
        }
    }
}

impl<E, R, L: PinnedLogs> DkgAgree<E, R, L> {
    /// The proposal this node puts to the instance at `view`, waiting inside the
    /// leader's own window until it has one.
    ///
    /// The two node-local inputs — the recorded dealer-log set and the covering
    /// share-confirmations — do not exist yet at the instant the instance spawns:
    /// the instance is announced on the ceremony's seal edge, and the peers' sealed
    /// logs and the confirmations minted from them are still in flight. A leader
    /// that answered once and then parked spent its whole `leader_timeout` holding
    /// an answer that went stale in milliseconds. Measured on the docker stand
    /// (2026-08-19, `case growth`, two committee changes): `view = 2` both times and
    /// `T_agree` = 30.07 s / 30.09 s, i.e. exactly one `LEADER_TIMEOUT`, view 1
    /// nullified with nothing proposed. Re-reading on the growth edge is what makes
    /// view 1 decide.
    ///
    /// Park-don't-error is UNCHANGED and is why the retry is expressible at all:
    /// resolving the request as an error trips the leader deadline at once, so the
    /// window a late dealer log has to arrive in only exists while the request is
    /// held open. The wait ends by itself when the view moves on — the voter drops
    /// the receiver, and [`drive`] cancels on that.
    ///
    /// It wakes on [`ConfirmPool::subscribe`] rather than on a clock: a poll
    /// interval would be a second timeout to tune beside `leader_timeout`, and the
    /// edge it would sample is already published by the writer.
    ///
    /// The FIRST admissible set is proposed, not the widest one. Waiting for the
    /// set to stop growing would be the quiescence timer this design does not have,
    /// and a narrower pinned set is not a weaker one: `derive_pinned` needs a
    /// quorum, and a set that stalls below the full committee is exactly the case
    /// where pinning what is actually there is the right answer. In practice the
    /// dealers seal on one height tick and the set fills before any confirmation
    /// covering it circulates, so the first admissible set is the full one.
    async fn build_proposal(self, view: View) -> Option<DkgProposal>
    where
        Self: Clone,
    {
        if self.committee.is_empty() {
            debug!(
                epoch = self.target_epoch,
                "dkg agree: no committee to propose against"
            );
            self.notes.refuse(self.target_epoch, view, "no_committee");
            return None;
        }
        // Subscribed BEFORE the first attempt: a growth landing between the two is
        // still delivered, so the leader can never sleep holding a usable answer.
        let mut grew = self.confirms.subscribe();
        loop {
            if let Some(proposal) = self.clone().attempt_proposal(view).await {
                return Some(proposal);
            }
            // Only the pool's own clones keep the sender alive, and this task holds
            // one; an error here means the runtime is tearing down.
            if grew.changed().await.is_err() {
                return None;
            }
        }
    }

    /// One attempt at [`Self::build_proposal`]'s value. Records why it refused, on
    /// the warn-once ledger, so a retry loop cannot turn a stall into a log flood.
    ///
    /// The confirmations attached are exactly those that COVER the set being
    /// proposed. A confirmation of a narrower set says nothing about a member's
    /// ability to finalize over this one, so counting it would inflate the very
    /// number the bar exists to measure.
    ///
    /// Runs inside the spawned propose task, not on the voter's own loop — `derive`
    /// is a round-trip to the ceremony owner, and the voter must not block on it.
    async fn attempt_proposal(self, view: View) -> Option<DkgProposal> {
        let n = self.committee.len();
        let target_epoch = self.target_epoch;
        let local = local_set(&self.recorded, target_epoch, n);
        let quorum = N3f1::quorum(n) as usize;
        if local.len() < quorum {
            debug!(
                epoch = target_epoch,
                held = local.len(),
                quorum,
                "dkg agree: below quorum, not proposing"
            );
            self.notes.refuse(target_epoch, view, "quorum_not_met");
            return None;
        }
        let logs: Vec<(u8, B256)> = local.iter().map(|(idx, hash)| (*idx, *hash)).collect();
        let confirms = self.confirms.covering(target_epoch, &logs);
        let bar = entry_bar(n, view);
        if confirms.len() < bar {
            self.notes
                .bar_unmet(target_epoch, view, confirms.len(), quorum, bar);
            return None;
        }
        match self.pinned.derive(local).await {
            PinnedDerive::Derived(group_key) => Some(DkgProposal {
                target_epoch,
                logs,
                group_key: *group_key,
                confirms,
            }),
            other => {
                debug!(
                    epoch = target_epoch,
                    outcome = ?other,
                    "dkg agree: cannot derive the key over our own set, not proposing"
                );
                self.notes
                    .refuse(target_epoch, view, "no_key_over_local_set");
                None
            }
        }
    }
}

/// The clauses of the acceptance predicate that need nothing but the proposal,
/// the committee, the network's confirmation namespace and the view. Every one of
/// them is a permanent property of the proposal under `committee[target_epoch]`,
/// so two honest nodes holding the same proposal always reach the same answer —
/// which is what makes rejecting on them safe. The view belongs in that list
/// rather than beside it: it is agreed by the protocol and carried in every vote,
/// so it is the one moving part every honest node in a round reads identically.
/// Everything that depends on what THIS node happens to hold is decided in
/// [`DkgAgree::decide`], and parks there.
fn rejects_structurally(
    proposal: &DkgProposal,
    target_epoch: u64,
    committee: &[PeerPubkey],
    namespace: &[u8],
    view: View,
) -> bool {
    let n = committee.len();
    if proposal.target_epoch != target_epoch {
        warn!(
            epoch = target_epoch,
            claimed = proposal.target_epoch,
            "dkg agree: proposal names another epoch"
        );
        return true;
    }
    if let Some((idx, _)) = proposal.logs.iter().find(|(idx, _)| *idx as usize >= n) {
        warn!(
            epoch = target_epoch,
            idx, n, "dkg agree: proposal names a seat outside the committee"
        );
        return true;
    }
    let quorum = N3f1::quorum(n) as usize;
    if proposal.logs.len() < quorum {
        warn!(
            epoch = target_epoch,
            pinned = proposal.logs.len(),
            quorum,
            "dkg agree: proposal pins fewer dealers than the quorum"
        );
        return true;
    }
    for confirm in &proposal.confirms {
        if !confirm_is_countable(confirm, namespace, committee, target_epoch) {
            warn!(
                epoch = target_epoch,
                idx = confirm.idx,
                claimed = confirm.target_epoch,
                "dkg agree: proposal carries a share-confirmation the named member did not sign \
                 for this epoch and this set"
            );
            return true;
        }
        // The signature fixes WHICH set the member attested; this fixes that the
        // set attested covers the one being agreed. Without it a confirmation of a
        // narrower set would count towards a proposal its signer cannot finalize
        // over.
        if !confirm.covers(&proposal.logs) {
            warn!(
                epoch = target_epoch,
                idx = confirm.idx,
                "dkg agree: proposal counts a share-confirmation that does not cover its own \
                 pinned set"
            );
            return true;
        }
    }
    let bar = entry_bar(n, view);
    if proposal.confirms.len() < bar {
        // Both arms are rejections; they are separated because they mean different
        // things about the network. Below the QUORUM the members are genuinely
        // absent and the margin release will not help; between quorum and the bar
        // the epoch starts on its own at `MARGIN_RELEASE_VIEW`.
        let quorum = N3f1::quorum(n) as usize;
        if proposal.confirms.len() < quorum {
            warn!(
                epoch = target_epoch,
                view = view.get(),
                confirms = proposal.confirms.len(),
                quorum,
                "dkg agree: proposal is confirmed by fewer members than the quorum"
            );
        } else {
            warn!(
                epoch = target_epoch,
                view = view.get(),
                confirms = proposal.confirms.len(),
                bar,
                release_view = MARGIN_RELEASE_VIEW,
                "dkg agree: proposal clears the quorum but not the entry-bar margin"
            );
        }
        return true;
    }
    false
}

/// Decide `payload`, or park forever.
///
/// Parks (never resolves) on: a body engine that has gone away, an unreadable
/// committee, and — the case this whole function exists for — a pinned dealer
/// log whose body this node does not hold. That last one is a delivery race, not
/// a bad proposal: the body arrives over the existing `{epoch, dealer}`
/// resolver, and a later round proposing the same set is accepted. Answering it
/// with `false` would nullify the view instead.
impl<E, R, L> DkgAgree<E, R, L>
where
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
    L: PinnedLogs,
{
    async fn decide(mut self, parent: (View, Digest), view: View, payload: Digest) -> Verdict {
        let target_epoch = self.target_epoch;
        if let Some(certified) = certified_value(parent) {
            if payload != certified {
                warn!(
                    epoch = target_epoch,
                    "dkg agree: proposal replaces a value this instance already certified"
                );
                return Verdict::Reject;
            }
        }
        let Ok(proposal) = self.bodies.subscribe(payload).await.await else {
            // The body engine is gone (teardown). Not a verdict.
            return Verdict::Park;
        };
        if self.committee.is_empty() {
            warn!(
                epoch = target_epoch,
                "dkg agree: no committee to verify against"
            );
            return Verdict::Park;
        }
        if rejects_structurally(
            &proposal,
            target_epoch,
            &self.committee,
            self.confirms.namespace(),
            view,
        ) {
            return Verdict::Reject;
        }

        let set: BTreeMap<u8, B256> = proposal.logs.iter().copied().collect();
        match self.pinned.derive(set).await {
            PinnedDerive::Derived(group_key) => {
                if *group_key == proposal.group_key {
                    return Verdict::Accept;
                }
                warn!(
                    epoch = target_epoch,
                    "dkg agree: proposal's group key is not the one its own pinned set derives"
                );
                Verdict::Reject
            }
            PinnedDerive::Unusable => {
                warn!(
                    epoch = target_epoch,
                    "dkg agree: no key follows from the proposal's pinned set"
                );
                Verdict::Reject
            }
            PinnedDerive::Missing(indices) => {
                debug!(
                    epoch = target_epoch,
                    ?indices,
                    "dkg agree: parking verify on missing dealer-log bodies"
                );
                fetch_bodies(&mut self.logs, &self.committee, target_epoch, &indices).await;
                Verdict::Park
            }
            PinnedDerive::Unavailable => {
                debug!(
                    epoch = target_epoch,
                    "dkg agree: parking verify, no ceremony state to decide against"
                );
                Verdict::Park
            }
        }
    }
}

async fn fetch_bodies<R>(logs: &mut R, committee: &[PeerPubkey], epoch: u64, indices: &[u8])
where
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
{
    let Ok(targets) = NonEmptyVec::try_from(committee.to_vec()) else {
        return;
    };
    let requests: Vec<_> = indices
        .iter()
        .filter_map(|idx| committee.get(*idx as usize))
        .map(|dealer| {
            (
                DkgLogKey {
                    epoch,
                    dealer: dealer.clone(),
                },
                targets.clone(),
            )
        })
        .collect();
    if !requests.is_empty() {
        logs.fetch_all_targeted(requests).await;
    }
}

impl<E, R, L> Automaton for DkgAgree<E, R, L>
where
    E: Spawner + Metrics + Clock + Clone + Send + Sync + 'static,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
    L: PinnedLogs,
{
    type Context = SimplexContext<Digest, PeerPubkey>;
    type Digest = Digest;

    async fn genesis(&mut self, epoch: Epoch) -> Digest {
        agreement_genesis(epoch.get())
    }

    async fn propose(&mut self, context: Self::Context) -> oneshot::Receiver<Digest> {
        let build = self.clone().build_proposal(context.round.view());
        let certified = certified_value(context.parent);
        let bodies = self.bodies.clone();
        let last_built = self.last_built.clone();
        let round = context.round;
        let (tx, rx) = oneshot::channel();
        self.context
            .clone()
            .with_label("dkg_propose")
            .with_attribute("round", context.round)
            .spawn(move |_| async move {
                let decision = async move {
                    // Nothing to say, or nowhere to stash the body the relay will
                    // ask for: hold the request open so the view runs its leader
                    // timeout out. Resolving it as an error would trip the timeout
                    // immediately, which buys nothing and shortens the window a
                    // late dealer log has to arrive in.
                    let proposal = match certified {
                        // A value is already certified in this instance, so this
                        // view has nothing to decide: re-propose it verbatim. The
                        // same bar is enforced on the receiving side in `decide`,
                        // and a leader that ignored it here would only get its own
                        // view nullified.
                        Some(digest) => {
                            let Ok(proposal) = bodies.subscribe(digest).await.await else {
                                return Decision::Park;
                            };
                            proposal
                        }
                        None => {
                            let Some(proposal) = build.await else {
                                return Decision::Park;
                            };
                            proposal
                        }
                    };
                    let digest = proposal.digest();
                    if !last_built.arm(round, proposal) {
                        return Decision::Park;
                    }
                    Decision::Resolve(digest)
                };
                drive(tx, decision).await;
            });
        rx
    }

    async fn verify(&mut self, context: Self::Context, payload: Digest) -> oneshot::Receiver<bool> {
        let verdict = self
            .clone()
            .decide(context.parent, context.round.view(), payload);
        // The ONE place a `Verdict` becomes the voter's two-valued answer plus a park.
        let decision = async move { Decision::from(verdict.await) };
        let (tx, rx) = oneshot::channel();
        self.context
            .clone()
            .with_label("dkg_verify")
            .with_attribute("round", context.round)
            .spawn(move |_| async move {
                drive(tx, decision).await;
            });
        rx
    }
}

impl<E, R, L> CertifiableAutomaton for DkgAgree<E, R, L>
where
    E: Spawner + Metrics + Clock + Clone + Send + Sync + 'static,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
    L: PinnedLogs,
{
    /// Unconditionally certifiable. The ordering plane uses this hook for an
    /// availability gate (a block must be fetchable before it may finalize) and
    /// for the boundary seed-verify; this instance has neither. Its payload is
    /// already fully checked at `verify`, and there is no execution behind it to
    /// wait for.
    async fn certify(
        &mut self,
        _round: commonware_consensus::types::Round,
        _payload: Digest,
    ) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        tx.send_lossy(true);
        rx
    }
}

impl<E, R, L> Relay for DkgAgree<E, R, L>
where
    E: Clone + Send + Sync + 'static,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
    L: PinnedLogs,
{
    type Digest = Digest;
    type PublicKey = PeerPubkey;
    type Plan = Plan<PeerPubkey>;

    async fn broadcast(&mut self, payload: Digest, plan: Plan<PeerPubkey>) {
        match plan {
            Plan::Propose => {
                let Some(proposal) = self.last_built.take() else {
                    warn!(
                        epoch = self.target_epoch,
                        "dkg agree: no built proposal to broadcast"
                    );
                    return;
                };
                if proposal.digest() != payload {
                    warn!(
                        epoch = self.target_epoch,
                        "dkg agree: built proposal does not match the digest consensus proposed"
                    );
                    return;
                }
                drop(self.bodies.broadcast(Recipients::All, proposal).await);
            }
            Plan::Forward { peers, .. } => {
                if peers.is_empty() {
                    return;
                }
                // Forwarding serves a peer that voted on a digest whose body it
                // never received, so it reads from the buffer rather than from
                // `last_built`: the body being forwarded is usually someone
                // else's.
                let Some(proposal) = self.bodies.get(payload).await else {
                    debug!(
                        epoch = self.target_epoch,
                        "dkg agree: asked to forward a body we do not hold"
                    );
                    return;
                };
                drop(
                    self.bodies
                        .broadcast(Recipients::Some(peers), proposal)
                        .await,
                );
            }
        }
    }
}

/// Count and name the body-checked dealer logs this node holds that the agreed set
/// left out.
///
/// OBSERVABILITY ONLY, and it must stay that way. This is the signal that separates
/// a rare delivery race from a proposer systematically dropping entries, but the
/// acceptance predicate must never read it: a predicate that consulted local state
/// would turn every delivery race into a lost view, which is the failure the whole
/// predicate is written to avoid — and which the ordering plane's own verify forbids
/// by rule.
pub(crate) fn note_omissions(
    recorded: &DkgLogIndex,
    target_epoch: u64,
    committee_len: usize,
    metrics: &BeaconMetrics,
    proposal: &DkgProposal,
) {
    let local = local_set(recorded, target_epoch, committee_len);
    let agreed: BTreeSet<u8> = proposal.logs.iter().map(|(idx, _)| *idx).collect();
    let omitted: Vec<u8> = local
        .keys()
        .copied()
        .filter(|idx| !agreed.contains(idx))
        .collect();
    if omitted.is_empty() {
        return;
    }
    metrics.dkg_agree_logs_omitted.inc_by(omitted.len() as u64);
    warn!(
        epoch = target_epoch,
        ?omitted,
        agreed = proposal.logs.len(),
        "dkg agree: the agreed set omits dealer logs this node holds"
    );
}

/// The agreement instance's `Reporter`.
///
/// Deliberately NOT the marshal, the slasher or `spec_exec`. Agreement rounds are
/// not consensus rounds: feeding them to the slasher would manufacture
/// equivocation evidence about `Round(target_epoch, v)` pairs that the ordering
/// plane never held. The consequence — equivocation INSIDE this plane is
/// unslashable — is a deliberate, bounded cost: a double-signer buys nullified
/// views, never a wrong agreed value, because the acceptance predicate and the
/// quorum still decide what gets certified.
///
/// It forwards the CERTIFICATE and never the body. Resolving the body here would
/// have to happen on the voter's own chain — journal replay awaits every
/// `report` inline (`CW/consensus/src/simplex/actors/voter/actor.rs:786-793`) — so
/// the only thing that fits is an instant cache peek, and after a restart the
/// in-memory buffer is empty. That peek returning `None` used to leave the
/// supervisor waiting on a verdict that could never come: the instance was never
/// torn down and its journal partition was never reclaimed. The body is resolved by
/// the supervisor instead, off this chain and with an await that a late body can
/// actually satisfy ([`crate::beacon::dkg_engine`]).
pub(crate) struct DkgReporter {
    target_epoch: u64,
    verdict: tokio::sync::mpsc::Sender<Finalization<BlsScheme, Digest>>,
    /// Journal replay re-fires every reported activity, so the certificate leaves
    /// at most once.
    delivered: Arc<AtomicBool>,
}

impl Clone for DkgReporter {
    fn clone(&self) -> Self {
        Self {
            target_epoch: self.target_epoch,
            verdict: self.verdict.clone(),
            delivered: self.delivered.clone(),
        }
    }
}

impl DkgReporter {
    pub fn new(
        target_epoch: u64,
        verdict: tokio::sync::mpsc::Sender<Finalization<BlsScheme, Digest>>,
    ) -> Self {
        Self {
            target_epoch,
            verdict,
            delivered: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Reporter for DkgReporter {
    type Activity = Activity<BlsScheme, Digest>;

    async fn report(&mut self, activity: Self::Activity) {
        let Activity::Finalization(finalization) = activity else {
            return;
        };
        if self.delivered.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.verdict.send(finalization).await.is_err() {
            warn!(
                epoch = self.target_epoch,
                "dkg agree: nothing left to take the agreed certificate"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_cryptography::{
        bls12381::{dkg::deal, primitives::sharing::Mode, primitives::variant::MinSig},
        ed25519::PrivateKey as Ed25519PrivateKey,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1};
    use fluentbase_bls::PeerPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// A park HOLDS the voter's sender; only a decision sends, and nothing drops it
    /// early.
    ///
    /// This is the fork-critical property of the whole plane and it is invisible to
    /// the agreement tests, which only ever see decided rounds. `try_recv` is the
    /// discriminator: a live-but-unsent sender reads `Empty`, a dropped one reads
    /// `Closed` — and `Closed` reaches the voter as `TimeoutReason::IgnoredProposal`,
    /// the same lost view a wrong `false` would have cost.
    ///
    /// Both arms run through the real `drive`, so the `Empty` assertion cannot be
    /// vacuous: the `Resolve` half proves the same channel does deliver when asked.
    #[tokio::test]
    async fn a_park_holds_the_sender_and_a_decision_sends_it() {
        // Park: no value, and the channel stays open for as long as the voter wants it.
        let (tx, mut rx) = oneshot::channel::<bool>();
        let parked = tokio::spawn(drive(tx, async { Decision::Park }));
        tokio::task::yield_now().await;
        assert!(
            matches!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "a park must leave the sender live and unsent — dropping it nullifies the \
             view, and sending anything decides a round this node cannot decide"
        );

        // ... and it exits cleanly once the view moves on, rather than leaking a task.
        drop(rx);
        parked
            .await
            .expect("drive must return when the voter drops its receiver");

        // Decision: the same driver delivers. Without this the assertion above would
        // hold on a `drive` that had simply stopped working.
        let (tx, mut rx) = oneshot::channel::<bool>();
        drive(tx, async { Decision::Resolve(false) }).await;
        assert_eq!(
            rx.try_recv(),
            Ok(false),
            "a decided verdict must reach the voter verbatim"
        );
    }

    fn fixture() -> DkgProposal {
        let mut rng = StdRng::seed_from_u64(11);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..5).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (group_key, _shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        let signer = Ed25519PrivateKey::random(&mut rng);

        DkgProposal {
            target_epoch: 42,
            logs: vec![
                (0, B256::repeat_byte(0xA1)),
                (2, B256::repeat_byte(0xB2)),
                (4, B256::repeat_byte(0xC3)),
            ],
            group_key,
            confirms: vec![
                ShareConfirm {
                    idx: 1,
                    target_epoch: 42,
                    recorded: vec![(0, B256::repeat_byte(0xA1)), (2, B256::repeat_byte(0xB2))],
                    sig: signer.sign(b"ns", b"confirm-one"),
                },
                ShareConfirm {
                    idx: 3,
                    target_epoch: 42,
                    recorded: vec![],
                    sig: signer.sign(b"ns", b"confirm-two"),
                },
            ],
        }
    }

    #[test]
    fn proposal_round_trips_and_sizes_match() {
        let proposal = fixture();
        let encoded = proposal.encode();
        assert_eq!(proposal.encode_size(), encoded.len());
        let decoded = DkgProposal::decode(encoded.clone()).expect("decode");
        assert_eq!(decoded, proposal);
        assert_eq!(decoded.digest(), proposal.digest());
        assert_eq!(proposal.commitment(), proposal.digest());
    }

    #[test]
    fn read_rejects_trailing_bytes() {
        let mut bytes = fixture().encode().to_vec();
        bytes.push(0xFF);
        assert!(matches!(
            DkgProposal::decode(bytes.as_slice()),
            Err(Error::ExtraData(_))
        ));
    }

    #[test]
    fn read_rejects_descending_logs() {
        let mut proposal = fixture();
        proposal.logs.reverse();
        let err = DkgProposal::decode(proposal.encode()).expect_err("descending logs");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    #[test]
    fn read_rejects_duplicate_log_idx() {
        let mut proposal = fixture();
        proposal.logs = vec![(1, B256::repeat_byte(0x11)), (1, B256::repeat_byte(0x22))];
        let err = DkgProposal::decode(proposal.encode()).expect_err("duplicate log idx");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    #[test]
    fn read_rejects_log_idx_beyond_max_committee_size() {
        let mut proposal = fixture();
        proposal.logs = vec![(MAX_COMMITTEE_SIZE as u8, B256::repeat_byte(0x33))];
        let err = DkgProposal::decode(proposal.encode()).expect_err("out-of-range log idx");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    #[test]
    fn read_rejects_descending_confirms() {
        let mut proposal = fixture();
        proposal.confirms.reverse();
        let err = DkgProposal::decode(proposal.encode()).expect_err("descending confirms");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    #[test]
    fn read_rejects_duplicate_confirm_idx() {
        let mut proposal = fixture();
        let first = proposal.confirms[0].clone();
        proposal.confirms = vec![first.clone(), first];
        let err = DkgProposal::decode(proposal.encode()).expect_err("duplicate confirm idx");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    #[test]
    fn read_rejects_non_canonical_recorded_set() {
        let mut proposal = fixture();
        proposal.confirms[0].recorded.reverse();
        let err = DkgProposal::decode(proposal.encode()).expect_err("descending recorded");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    /// A count above the cap is rejected on the prefix alone — before any
    /// allocation and before the rest of the frame is even read.
    #[test]
    fn read_rejects_oversize_log_count() {
        let mut bytes = Vec::new();
        7u64.write(&mut bytes);
        (MAX_COMMITTEE_SIZE as u32 + 1).write(&mut bytes);
        let err = DkgProposal::decode(bytes.as_slice()).expect_err("oversize log count");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    #[test]
    fn read_rejects_oversize_confirm_count_and_group_key() {
        let proposal = fixture();
        let key = encode_outcome(&proposal.group_key);

        let mut oversize_key = Vec::new();
        proposal.target_epoch.write(&mut oversize_key);
        write_log_set(&proposal.logs, &mut oversize_key);
        (MAX_BEACON_OUTCOME_SIZE as u32 + 1).write(&mut oversize_key);
        let err = DkgProposal::decode(oversize_key.as_slice()).expect_err("oversize group_key");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");

        let mut oversize_confirms = Vec::new();
        proposal.target_epoch.write(&mut oversize_confirms);
        write_log_set(&proposal.logs, &mut oversize_confirms);
        (key.len() as u32).write(&mut oversize_confirms);
        oversize_confirms.extend_from_slice(&key);
        (MAX_COMMITTEE_SIZE as u32 + 1).write(&mut oversize_confirms);
        let err =
            DkgProposal::decode(oversize_confirms.as_slice()).expect_err("oversize confirm count");
        assert!(matches!(err, Error::Invalid("dkg_agree", _)), "{err:?}");
    }

    /// The entry bar at the committee sizes that actually run, and the release.
    ///
    /// The `f = 1` rows are the point: at 4, 5 and 6 seats the margin is 0, so the
    /// dpos smoke, the production path and the growth case's start state see the
    /// bare quorum and nothing changes for them. Any nonzero margin there would
    /// mean "every member must succeed", which is stricter, not sturdier.
    #[test]
    fn the_entry_bar_is_inert_where_f_is_one() {
        let held = View::new(1);
        let released = View::new(MARGIN_RELEASE_VIEW);
        for (n, f, quorum, margin) in [
            (4, 1, 3, 0),
            (5, 1, 4, 0),
            (6, 1, 5, 0),
            (7, 2, 5, 1),
            (10, 3, 7, 1),
            (51, 16, 35, 2),
        ] {
            assert_eq!(N3f1::max_faults(n) as usize, f, "n = {n}");
            assert_eq!(N3f1::quorum(n) as usize, quorum, "n = {n}");
            assert_eq!(entry_bar(n, held), quorum + margin, "n = {n}");
            assert_eq!(
                entry_bar(n, released),
                quorum,
                "the release must drop to the bare quorum at n = {n}"
            );
            assert_eq!(
                entry_bar(n, View::new(MARGIN_RELEASE_VIEW - 1)),
                quorum + margin,
                "the margin must still hold in the view before the release, n = {n}"
            );
        }
    }

    /// The invariant a configuration knob would break, asserted across every
    /// committee size the network can have: the bar never asks for more members than
    /// the committee has, and never for fewer than the quorum. `m <= f` is what buys
    /// the first half, and it follows from the fault model rather than from a
    /// setting — which is why neither `f` nor the margin is a parameter of
    /// [`entry_bar`]. A configurable margin would be a liveness split: the strictest
    /// node self-removes, and `> f` strict nodes halt the epoch.
    #[test]
    fn the_entry_bar_is_satisfiable_at_every_committee_size() {
        for n in 1..=MAX_COMMITTEE_SIZE as usize {
            let quorum = N3f1::quorum(n) as usize;
            assert!(
                margin(N3f1::max_faults(n)) <= N3f1::max_faults(n),
                "n = {n}"
            );
            for view in [1, 2, MARGIN_RELEASE_VIEW, 1_000] {
                let bar = entry_bar(n, View::new(view));
                assert!(bar >= quorum, "the bar fell below the quorum at n = {n}");
                assert!(bar <= n, "the bar is unsatisfiable at n = {n}");
            }
            assert!(
                entry_bar(n, View::new(MARGIN_RELEASE_VIEW)) <= entry_bar(n, View::new(1)),
                "the bar must be non-increasing in the view, or a value certified while \
                 the margin held could stop being acceptable"
            );
        }
        assert_eq!(
            entry_bar(0, View::new(1)),
            0,
            "an empty committee has no bar to clear"
        );
    }

    /// The confirmation's domain separation. The same ed25519 key signs dealer logs
    /// and player acks inside the ceremony, so a confirmation that verified under a
    /// neighbouring namespace would be a signature reused across contexts.
    #[test]
    fn a_confirmation_does_not_verify_across_namespaces() {
        let mut rng = StdRng::seed_from_u64(51);
        let key = Ed25519PrivateKey::random(&mut rng);
        let base = b"FLUENT_DPOS_V1_TEST";
        let pool = ConfirmPool::new(base);
        assert_ne!(pool.namespace(), base, "the pool must add its own domain");

        let confirm = ShareConfirm::sign(
            pool.namespace(),
            &key,
            0,
            7,
            vec![(0, B256::repeat_byte(0x01))],
        );
        assert!(confirm.verify(pool.namespace(), &key.public_key()));
        assert!(
            !confirm.verify(base, &key.public_key()),
            "a confirmation must not verify under the base namespace"
        );
        assert!(!confirm.verify(ConfirmPool::new(b"OTHER").namespace(), &key.public_key()));
    }

    mod agreement {
        use super::*;
        use commonware_consensus::{
            simplex::types::{Finalize, Nullify, Proposal},
            types::{Round, View},
        };
        use commonware_p2p::{
            simulated::{Config as SimConfig, Network},
            Manager as _,
        };
        use commonware_parallel::Sequential;
        use commonware_runtime::{deterministic, Runner as _};
        use commonware_utils::{ordered::BiMap, vec::NonEmptyVec, NZUsize, TryCollect as _};
        use fluentbase_bls::{
            fluent_namespace,
            keys::ValidatorBlsKeypair,
            scheme::{build_signer, build_verifier},
            BlsPubkey,
        };
        use std::{collections::BTreeSet, sync::RwLock, time::Duration};

        const TARGET: u64 = 9;
        /// `N3f1` at `n = 5` tolerates one fault, so four pinned dealers clear the
        /// quorum and three do not.
        const N: usize = 5;

        #[derive(Clone)]
        struct MockPinned(Arc<Mutex<PinnedDerive>>);

        impl MockPinned {
            fn new(answer: PinnedDerive) -> Self {
                Self(Arc::new(Mutex::new(answer)))
            }
        }

        impl PinnedLogs for MockPinned {
            async fn derive(&self, _pinned: BTreeMap<u8, B256>) -> PinnedDerive {
                self.0.lock().unwrap().clone()
            }
        }

        /// A `PinnedLogs` whose FIRST caller parks inside `derive` until released
        /// and whose later callers answer at once — a `derive` round-trip that
        /// outlives its own view, which is the only way one propose task can still
        /// be running when a later round's task has already built.
        #[derive(Clone)]
        struct GatedPinned {
            answer: PinnedDerive,
            open: Arc<watch::Sender<bool>>,
            calls: Arc<std::sync::atomic::AtomicUsize>,
        }

        impl GatedPinned {
            fn new(answer: PinnedDerive) -> Self {
                Self {
                    answer,
                    open: Arc::new(watch::channel(false).0),
                    calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                }
            }

            fn release(&self) {
                self.open.send_replace(true);
            }
        }

        impl PinnedLogs for GatedPinned {
            async fn derive(&self, _pinned: BTreeMap<u8, B256>) -> PinnedDerive {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    let mut open = self.open.subscribe();
                    while !*open.borrow_and_update() {
                        if open.changed().await.is_err() {
                            return PinnedDerive::Unavailable;
                        }
                    }
                }
                self.answer.clone()
            }
        }

        /// Records the keys the automaton asked the dealer-log resolver for, so a
        /// parked `verify` can be shown to have ALSO driven the repair that ends
        /// the park.
        #[derive(Clone, Default)]
        struct RecordingResolver(Arc<Mutex<BTreeSet<DkgLogKey>>>);

        impl Resolver for RecordingResolver {
            type Key = DkgLogKey;
            type PublicKey = PeerPubkey;
            async fn fetch(&mut self, key: Self::Key) {
                self.0.lock().unwrap().insert(key);
            }
            async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
                self.0.lock().unwrap().extend(keys);
            }
            async fn fetch_targeted(&mut self, key: Self::Key, _: NonEmptyVec<Self::PublicKey>) {
                self.0.lock().unwrap().insert(key);
            }
            async fn fetch_all_targeted(
                &mut self,
                requests: Vec<(Self::Key, NonEmptyVec<Self::PublicKey>)>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .extend(requests.into_iter().map(|(k, _)| k));
            }
            async fn cancel(&mut self, key: Self::Key) {
                self.0.lock().unwrap().remove(&key);
            }
            async fn clear(&mut self) {
                self.0.lock().unwrap().clear();
            }
            async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
                self.0.lock().unwrap().retain(|k| predicate(k));
            }
        }

        /// A four-member signing committee for the agreement instance's own
        /// certificates. Separate from `committee_of` above, which only supplies
        /// the peer identities the dealer-log index is numbered against.
        fn signing_committee(seed: u64) -> (Vec<BlsScheme>, BlsScheme) {
            let mut rng = StdRng::seed_from_u64(seed);
            let peer_sks: Vec<_> = (0..4)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect();
            let bls_kps: Vec<_> = (0..4)
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
            let ns = fluent_namespace(20_994);
            let signers = bls_kps
                .iter()
                .map(|kp| build_signer(&ns, bimap.clone(), kp, TARGET, None).expect("member"))
                .collect();
            (signers, build_verifier(&ns, bimap, TARGET, None))
        }

        fn agreement_round() -> Round {
            Round::new(Epoch::new(TARGET), View::new(1))
        }

        fn finalization_over(payload: Digest) -> Finalization<BlsScheme, Digest> {
            let (signers, verifier) = signing_committee(21);
            let proposal = Proposal::new(agreement_round(), View::new(0), payload);
            let finalizes: Vec<_> = signers
                .iter()
                .take(3)
                .map(|s| Finalize::sign(s, proposal.clone()).expect("sign"))
                .collect();
            Finalization::from_finalizes(&verifier, finalizes.iter(), &Sequential).expect("quorum")
        }

        fn a_nullify() -> Nullify<BlsScheme> {
            let (signers, _) = signing_committee(22);
            Nullify::sign::<Digest>(&signers[0], agreement_round()).expect("sign")
        }

        /// The committee a test runs over: the seats, the private keys that let it
        /// mint REAL share-confirmations for them, and the pool the automaton reads.
        /// Real signatures throughout — the entry bar is a signature check, so a
        /// fixture that faked them would test nothing.
        struct Committee {
            keys: Vec<Ed25519PrivateKey>,
            members: Vec<PeerPubkey>,
            pool: ConfirmPool,
        }

        impl Committee {
            fn new(n: usize, seed: u64) -> Self {
                let mut rng = StdRng::seed_from_u64(seed);
                let keys: Vec<Ed25519PrivateKey> = (0..n)
                    .map(|_| Ed25519PrivateKey::random(&mut rng))
                    .collect();
                let members = keys
                    .iter()
                    .map(commonware_cryptography::Signer::public_key)
                    .collect();
                Self {
                    keys,
                    members,
                    pool: ConfirmPool::new(b"FLUENT_TEST_AGREE"),
                }
            }

            /// The entry bar at view 1 — where the margin, if any, still holds.
            fn bar(&self) -> usize {
                entry_bar(self.members.len(), View::new(1))
            }

            /// `count` confirmations of `logs`, signed by the first `count` seats.
            fn confirms(&self, logs: &[(u8, B256)], count: usize) -> Vec<ShareConfirm> {
                self.keys
                    .iter()
                    .take(count)
                    .enumerate()
                    .map(|(idx, key)| {
                        ShareConfirm::sign(
                            self.pool.namespace(),
                            key,
                            idx as u8,
                            TARGET,
                            logs.to_vec(),
                        )
                    })
                    .collect()
            }

            /// The same, recorded into the pool so a local `propose` finds them.
            fn seed_pool(&self, logs: &[(u8, B256)], count: usize) {
                for confirm in self.confirms(logs, count) {
                    assert!(
                        self.pool.record(&self.members, confirm),
                        "the pool refused a confirmation it should have taken"
                    );
                }
            }

            /// A proposal carrying exactly the confirmations the bar asks for.
            fn proposal(&self, logs: Vec<(u8, B256)>, key: DkgOutcome) -> DkgProposal {
                let confirms = self.confirms(&logs, self.bar());
                DkgProposal {
                    target_epoch: TARGET,
                    logs,
                    group_key: key,
                    confirms,
                }
            }
        }

        fn outcome(seed: u64) -> DkgOutcome {
            let mut rng = StdRng::seed_from_u64(seed);
            let players: Set<PeerPubkey> = Set::from_iter_dedup(
                (0..N).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()),
            );
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal")
                .0
        }

        /// Four pinned dealers over a five-seat committee: exactly the quorum.
        fn quorum_logs() -> Vec<(u8, B256)> {
            (0..4u8).map(|i| (i, B256::repeat_byte(0x10 + i))).collect()
        }

        fn index_holding(entries: &[(u8, B256)]) -> DkgLogIndex {
            let mut per_epoch = BTreeMap::new();
            per_epoch.insert(TARGET, entries.iter().copied().collect::<BTreeMap<_, _>>());
            Arc::new(RwLock::new(per_epoch))
        }

        /// A started body engine on a one-node simulated network, with the node in
        /// `latest.primary` so a locally broadcast body is retained (and therefore
        /// resolvable by `subscribe`, which is what lets the tests below isolate the
        /// dealer-log half of the predicate).
        async fn body_mailbox(context: &deterministic::Context, me: PeerPubkey) -> BodyMailbox {
            let (network, oracle) = Network::new(
                context.with_label("network"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            let channel = oracle
                .control(me.clone())
                .register(
                    fluentbase_p2p::constants::BROADCAST_CHANNEL,
                    fluentbase_p2p::constants::BROADCAST_QUOTA,
                )
                .await
                .expect("register BROADCAST_CHANNEL");
            oracle
                .manager()
                .track(0, Set::from_iter_dedup([me.clone()]))
                .await;
            let (engine, mailbox) = commonware_broadcast::buffered::Engine::new(
                context.with_label("dkg_bodies"),
                commonware_broadcast::buffered::Config {
                    public_key: me,
                    mailbox_size: 64,
                    deque_size: MAX_SET_LEN,
                    priority: true,
                    codec_config: (),
                    peer_provider: oracle.manager(),
                },
            );
            drop(engine.start(channel));
            mailbox
        }

        type Agree<L = MockPinned> = DkgAgree<deterministic::Context, RecordingResolver, L>;

        async fn agree_over<L: PinnedLogs>(
            context: &deterministic::Context,
            committee: Vec<PeerPubkey>,
            recorded: DkgLogIndex,
            pinned: L,
            resolver: RecordingResolver,
            confirms: ConfirmPool,
        ) -> (Agree<L>, BodyMailbox) {
            let bodies = body_mailbox(context, committee[0].clone()).await;
            let agree = DkgAgree::new(
                context.clone(),
                DkgAgreeConfig {
                    target_epoch: TARGET,
                    committee,
                    bodies: bodies.clone(),
                    logs: resolver,
                    recorded,
                    pinned,
                    confirms,
                    metrics: BeaconMetrics::default(),
                },
            );
            (agree, bodies)
        }

        /// How long a park is observed before it counts as a park. The decision
        /// path has no timer of its own — it either resolves off an await that is
        /// already satisfied or it is `pending()` forever — so this only has to
        /// outlast the mailbox round-trips, not any protocol timeout.
        const PARK_WINDOW: Duration = Duration::from_secs(5);

        /// Resolve `rx`, or report that it stayed pending.
        async fn settle(
            context: &deterministic::Context,
            rx: oneshot::Receiver<bool>,
        ) -> Option<bool> {
            tokio::select! {
                _ = context.sleep(PARK_WINDOW) => None,
                verdict = rx => verdict.ok(),
            }
        }

        /// The first view, where the margin (if the committee has one) still holds.
        const VIEW1: View = View::new(1);

        fn ctx_at(committee: &[PeerPubkey], view: View) -> SimplexContext<Digest, PeerPubkey> {
            SimplexContext {
                round: commonware_consensus::types::Round::new(Epoch::new(TARGET), view),
                leader: committee[0].clone(),
                parent: (View::new(0), agreement_genesis(TARGET)),
            }
        }

        fn ctx_for(committee: &[PeerPubkey]) -> SimplexContext<Digest, PeerPubkey> {
            ctx_at(committee, VIEW1)
        }

        /// Resolve a `propose` request, or report that it stayed pending.
        async fn settle_digest(
            context: &deterministic::Context,
            rx: oneshot::Receiver<Digest>,
        ) -> Option<Digest> {
            tokio::select! {
                _ = context.sleep(PARK_WINDOW) => None,
                digest = rx => digest.ok(),
            }
        }

        /// THE load-bearing behaviour of this component. A pinned dealer log whose
        /// body has not reached this node yet is a delivery race, not a bad
        /// proposal: `verify` must leave the request pending — and drive the fetch
        /// that ends the race — instead of resolving `false`, which the voter reads
        /// as `InvalidProposal` and turns into an immediate nullify.
        #[test]
        fn verify_parks_on_a_missing_dealer_log_body() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 1);
                let committee = seats.members.clone();
                let resolver = RecordingResolver::default();
                let pinned = MockPinned::new(PinnedDerive::Missing(vec![1, 3]));
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&[]),
                    pinned,
                    resolver.clone(),
                    seats.pool.clone(),
                )
                .await;

                let proposal = seats.proposal(quorum_logs(), outcome(2));
                let digest = proposal.digest();
                drop(bodies.broadcast(Recipients::All, proposal).await);

                let rx = agree.verify(ctx_for(&committee), digest).await;
                assert_eq!(
                    settle(&context, rx).await,
                    None,
                    "verify resolved on a missing body instead of parking"
                );

                let asked = resolver.0.lock().unwrap().clone();
                for idx in [1usize, 3] {
                    assert!(
                        asked.contains(&DkgLogKey {
                            epoch: TARGET,
                            dealer: committee[idx].clone(),
                        }),
                        "verify parked without asking the resolver for seat {idx}"
                    );
                }
            });
        }

        /// The same shape with no ceremony state to answer from at all — also a
        /// property of this node, also a park.
        #[test]
        fn verify_parks_when_no_ceremony_state_can_answer() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 3);
                let committee = seats.members.clone();
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&[]),
                    MockPinned::new(PinnedDerive::Unavailable),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;
                let proposal = seats.proposal(quorum_logs(), outcome(4));
                let digest = proposal.digest();
                drop(bodies.broadcast(Recipients::All, proposal).await);
                let rx = agree.verify(ctx_for(&committee), digest).await;
                assert_eq!(settle(&context, rx).await, None);
            });
        }

        #[test]
        fn verify_accepts_a_proposal_whose_own_set_derives_its_key() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 5);
                let committee = seats.members.clone();
                let key = outcome(6);
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&[]),
                    MockPinned::new(PinnedDerive::Derived(Box::new(key.clone()))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;
                let proposal = seats.proposal(quorum_logs(), key);
                let digest = proposal.digest();
                drop(bodies.broadcast(Recipients::All, proposal).await);
                // This node has received NO confirmation of its own: the entry bar
                // is met out of the proposal payload alone. That is the whole reason
                // the confirmations ride the payload — a predicate counting local
                // receive state would turn every delivery race into a lost view, and
                // this verdict would be `false` instead.
                assert!(seats.pool.covering(TARGET, &quorum_logs()).is_empty());
                let rx = agree.verify(ctx_for(&committee), digest).await;
                assert_eq!(settle(&context, rx).await, Some(true));
            });
        }

        /// The fail-fast cross-check: the key travels in the proposal but is not the
        /// agreed value, so a key that the proposal's own pinned set does not derive
        /// is permanently unacceptable to every honest node.
        #[test]
        fn verify_rejects_a_key_the_pinned_set_does_not_derive() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 7);
                let committee = seats.members.clone();
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&[]),
                    MockPinned::new(PinnedDerive::Derived(Box::new(outcome(8)))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;
                let proposal = seats.proposal(quorum_logs(), outcome(9));
                let digest = proposal.digest();
                drop(bodies.broadcast(Recipients::All, proposal).await);
                let rx = agree.verify(ctx_for(&committee), digest).await;
                assert_eq!(settle(&context, rx).await, Some(false));
            });
        }

        #[test]
        fn structural_rejections_are_pure_functions_of_the_proposal() {
            let seats = Committee::new(N, 25);
            let ns = seats.pool.namespace();
            let key = outcome(11);
            let good = seats.proposal(quorum_logs(), key.clone());
            assert!(!rejects_structurally(
                &good,
                TARGET,
                &seats.members,
                ns,
                VIEW1
            ));
            assert!(
                rejects_structurally(&good, TARGET + 1, &seats.members, ns, VIEW1),
                "a proposal for another epoch must be rejected"
            );

            let mut off_roster = good.clone();
            off_roster.logs.push((7, B256::repeat_byte(0x77)));
            assert!(rejects_structurally(
                &off_roster,
                TARGET,
                &seats.members,
                ns,
                VIEW1
            ));

            let short = DkgProposal {
                logs: quorum_logs().into_iter().take(3).collect(),
                ..good.clone()
            };
            assert!(rejects_structurally(
                &short,
                TARGET,
                &seats.members,
                ns,
                VIEW1
            ));

            // A confirmation nobody on the committee signed buys a proposer nothing:
            // the bar counts members, and an unverifiable signature names none.
            let mut signer = StdRng::seed_from_u64(26);
            let outsider = Ed25519PrivateKey::random(&mut signer);
            let mut forged = good.clone();
            forged.confirms[0] = ShareConfirm::sign(ns, &outsider, 0, TARGET, quorum_logs());
            assert!(
                rejects_structurally(&forged, TARGET, &seats.members, ns, VIEW1),
                "a confirmation the named seat did not sign must not count"
            );

            // A confirmation of a NARROWER set says nothing about this one: its
            // signer cannot finalize over the seat it never recorded.
            let mut narrow = good.clone();
            narrow.confirms[0] = ShareConfirm::sign(
                ns,
                &seats.keys[0],
                0,
                TARGET,
                quorum_logs().into_iter().take(2).collect(),
            );
            assert!(
                rejects_structurally(&narrow, TARGET, &seats.members, ns, VIEW1),
                "a confirmation that does not cover the pinned set must not count"
            );

            // And the bar itself: one confirmation short is a rejection, however
            // valid every confirmation carried is.
            let mut thin = good.clone();
            thin.confirms.pop();
            assert!(
                rejects_structurally(&thin, TARGET, &seats.members, ns, VIEW1),
                "a proposal below the entry bar must be rejected"
            );
        }

        /// THE replay the signed epoch exists to stop. The DKG envelope's
        /// `ceremony_epoch` is unsigned framing and does not travel with a
        /// confirmation once it is lifted into a proposal payload, so without the
        /// epoch inside the signed bytes an honest confirmation for `E` counts
        /// towards a proposal for `E'` whose pinned set it happens to cover — and the
        /// entry bar clears before that many members have confirmed anything for
        /// `E'`.
        #[test]
        fn a_confirmation_is_worthless_in_another_epoch() {
            let seats = Committee::new(N, 27);
            let ns = seats.pool.namespace();
            let honest = seats.confirms(&quorum_logs(), seats.bar());
            for confirm in &honest {
                assert!(
                    confirm_is_countable(confirm, ns, &seats.members, TARGET),
                    "a genuine confirmation must count in its own epoch"
                );
                assert!(
                    !confirm_is_countable(confirm, ns, &seats.members, TARGET + 1),
                    "a confirmation for one epoch must not count in another"
                );
            }

            // The same thing seen from the proposal: lifting the bytes verbatim into
            // a proposal for the next epoch does not make them count there.
            let lifted = DkgProposal {
                target_epoch: TARGET + 1,
                logs: quorum_logs(),
                group_key: outcome(28),
                confirms: honest,
            };
            assert!(
                rejects_structurally(&lifted, TARGET + 1, &seats.members, ns, VIEW1),
                "a proposal for E+1 must not be carried by confirmations for E"
            );
        }

        /// The other half of the same binding. The signature commits to a DIGEST of
        /// the confirmed set, so re-attaching it to an inflated `recorded` — the
        /// cheapest way to make a narrow confirmation "cover" a wide proposal — does
        /// not verify.
        #[test]
        fn a_signature_does_not_survive_an_inflated_recorded_set() {
            let seats = Committee::new(N, 29);
            let ns = seats.pool.namespace();
            let narrow: Vec<(u8, B256)> = quorum_logs().into_iter().take(2).collect();
            let honest = ShareConfirm::sign(ns, &seats.keys[0], 0, TARGET, narrow);
            assert!(honest.verify(ns, &seats.members[0]));

            let mut inflated = honest.clone();
            inflated.recorded = quorum_logs();
            assert!(
                inflated.covers(&quorum_logs()),
                "the inflated set is what a forger would need it to be"
            );
            assert!(
                !inflated.verify(ns, &seats.members[0]),
                "a signature re-attached to a wider recorded set must not verify"
            );
            assert!(!confirm_is_countable(&inflated, ns, &seats.members, TARGET));
        }

        /// The release is keyed on the simplex view number and on nothing else. All
        /// three probes run at the same instant of the deterministic clock, so a
        /// timer-driven release could not produce this result.
        #[test]
        fn the_bar_releases_on_the_view_number_and_not_on_a_clock() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                // Seven seats: `f = 2`, so the margin is 1 and quorum-only
                // confirmations are below the bar until the release view.
                const SEATS: usize = 7;
                let seats = Committee::new(SEATS, 31);
                let committee = seats.members.clone();
                let quorum = N3f1::quorum(SEATS) as usize;
                assert_eq!(entry_bar(SEATS, VIEW1), quorum + 1);
                assert_eq!(entry_bar(SEATS, View::new(MARGIN_RELEASE_VIEW)), quorum);

                let held: Vec<(u8, B256)> = (0..quorum as u8)
                    .map(|i| (i, B256::repeat_byte(0x10 + i)))
                    .collect();
                seats.seed_pool(&held, quorum);

                let (mut agree, _bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&held),
                    MockPinned::new(PinnedDerive::Derived(Box::new(outcome(32)))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let started = context.current();
                for view in 1..MARGIN_RELEASE_VIEW {
                    let rx = agree.propose(ctx_at(&committee, View::new(view))).await;
                    assert!(
                        settle_digest(&context, rx).await.is_none(),
                        "view {view} proposed while the margin was still unmet"
                    );
                }
                let rx = agree
                    .propose(ctx_at(&committee, View::new(MARGIN_RELEASE_VIEW)))
                    .await;
                assert!(
                    settle_digest(&context, rx).await.is_some(),
                    "the release view must propose on the same confirmations"
                );
                assert!(
                    context.current() > started,
                    "the probes advanced the clock, so the assertion above is about the view"
                );
            });
        }

        /// Below the bar the plane refuses, names WHICH of the two cases it is, and
        /// says it once per epoch per reason — a plane whose whole symptom is that
        /// views keep passing would otherwise make the log its own denial of service.
        #[test]
        fn an_unmet_bar_warns_once_per_reason_and_never_aborts() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                const SEATS: usize = 7;
                let seats = Committee::new(SEATS, 33);
                let committee = seats.members.clone();
                let quorum = N3f1::quorum(SEATS) as usize;
                let held: Vec<(u8, B256)> = (0..quorum as u8)
                    .map(|i| (i, B256::repeat_byte(0x10 + i)))
                    .collect();

                let metrics = BeaconMetrics::default();
                let bodies = body_mailbox(&context, committee[0].clone()).await;
                let mut agree = DkgAgree::new(
                    context.clone(),
                    DkgAgreeConfig {
                        target_epoch: TARGET,
                        committee: committee.clone(),
                        bodies,
                        logs: RecordingResolver::default(),
                        recorded: index_holding(&held),
                        pinned: MockPinned::new(PinnedDerive::Derived(Box::new(outcome(34)))),
                        confirms: seats.pool.clone(),
                        metrics: metrics.clone(),
                    },
                );

                // Nobody has confirmed: the quorum itself is unmet, and the release
                // will not help.
                for view in [1u64, 2] {
                    let rx = agree.propose(ctx_at(&committee, View::new(view))).await;
                    assert!(settle_digest(&context, rx).await.is_none());
                }
                assert!(
                    agree.reported().contains(&(TARGET, BAR_QUORUM_UNMET)),
                    "an absent quorum must be named as such"
                );
                assert_eq!(
                    metrics.dkg_agree_bar_unmet.get(),
                    1,
                    "the same reason must be reported once per epoch, not once per view"
                );

                // Now the quorum confirms but the margin does not: a different
                // reason, a different message, and one more report.
                seats.seed_pool(&held, quorum);
                let rx = agree.propose(ctx_at(&committee, VIEW1)).await;
                assert!(settle_digest(&context, rx).await.is_none());
                assert!(
                    agree.reported().contains(&(TARGET, BAR_MARGIN_UNMET)),
                    "a met quorum with an unmet margin is a different operator signal"
                );
                assert_eq!(metrics.dkg_agree_bar_unmet.get(), 2);

                // And the plane is still a plane: the release view proposes.
                let rx = agree
                    .propose(ctx_at(&committee, View::new(MARGIN_RELEASE_VIEW)))
                    .await;
                assert!(
                    settle_digest(&context, rx).await.is_some(),
                    "the plane must keep trying below the bar, never give up"
                );
            });
        }

        /// `propose` stashes what it built and `Relay` broadcasts exactly that, so
        /// the digest the voter names and the body peers receive are the same value.
        #[test]
        fn propose_stashes_the_body_relay_broadcasts() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 13);
                let committee = seats.members.clone();
                let key = outcome(14);
                let held: Vec<(u8, B256)> = quorum_logs();
                seats.seed_pool(&held, seats.bar());
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&held),
                    MockPinned::new(PinnedDerive::Derived(Box::new(key.clone()))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let rx = agree.propose(ctx_for(&committee)).await;
                let digest = tokio::select! {
                    _ = context.sleep(PARK_WINDOW) => panic!("propose parked"),
                    d = rx => d.expect("propose resolved"),
                };
                assert_eq!(digest, seats.proposal(held, key).digest());

                agree.broadcast(digest, Plan::Propose).await;
                let body = bodies.get(digest).await.expect("relay broadcast the body");
                assert_eq!(body.digest(), digest);
                assert_eq!(body.target_epoch, TARGET);
            });
        }

        /// A propose task that finishes after its own view is gone must not
        /// replace the body the current view's leader armed.
        ///
        /// `build_proposal` waits inside the leader's window, so a task asked at
        /// view 1 can still be inside `derive` when view 2 has already proposed
        /// and armed the slot. On an unkeyed slot that late write lands on top of
        /// view 2's body, `Relay::broadcast` then finds a digest consensus never
        /// asked for and refuses to send it, and view 2's peers park in `verify`
        /// on a body that never left — a whole certification timeout on the plane
        /// whose entire point is boundary latency.
        ///
        /// View 1's receiver is held to the end on purpose. Live, the voter drops
        /// it; holding it here fixes the order of the two writes instead of
        /// leaving it to a race the test would have to win.
        #[test]
        fn a_late_propose_task_cannot_replace_a_newer_rounds_body() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 41);
                let committee = seats.members.clone();
                let held: Vec<(u8, B256)> = quorum_logs();
                seats.seed_pool(&held, seats.bar());
                let pinned = GatedPinned::new(PinnedDerive::Derived(Box::new(outcome(42))));
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&held),
                    pinned.clone(),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let view1_rx = agree.propose(ctx_at(&committee, VIEW1)).await;
                context.sleep(Duration::from_millis(500)).await;

                // One confirmation more than view 1's task read, so the two bodies
                // are different values and a clobber is observable at all.
                for confirm in seats.confirms(&held, N).into_iter().skip(seats.bar()) {
                    assert!(seats.pool.record(&committee, confirm));
                }

                let rx = agree.propose(ctx_at(&committee, View::new(2))).await;
                let digest = settle_digest(&context, rx)
                    .await
                    .expect("view 2 must build over the wider confirmation set");

                pinned.release();
                context.sleep(Duration::from_millis(500)).await;

                agree.broadcast(digest, Plan::Propose).await;
                let body = bodies.get(digest).await.expect(
                    "the relay held back view 2's body: view 1's task overwrote it after the \
                     instance had left that view",
                );
                assert_eq!(body.digest(), digest);
                drop(view1_rx);
            });
        }

        /// A proposer that cannot justify a set stays silent and lets the view run
        /// its leader timeout out.
        #[test]
        fn propose_parks_below_quorum() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 15);
                let committee = seats.members.clone();
                let held: Vec<(u8, B256)> = quorum_logs().into_iter().take(3).collect();
                let (mut agree, _bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&held),
                    MockPinned::new(PinnedDerive::Derived(Box::new(outcome(16)))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;
                let rx = agree.propose(ctx_for(&committee)).await;
                let resolved = tokio::select! {
                    _ = context.sleep(PARK_WINDOW) => false,
                    _ = rx => true,
                };
                assert!(!resolved, "propose resolved with fewer dealers than quorum");
            });
        }

        /// A leader that could not build when it was asked must build when the
        /// missing input lands, INSIDE its own view.
        ///
        /// This is the shape the first live run measured (2026-08-19, `case
        /// growth`): the instance is announced on the ceremony's seal edge, so at
        /// the instant view 1's leader is asked, the dealer logs and the
        /// confirmations minted from them are still in flight. Answering once and
        /// parking cost a full `LEADER_TIMEOUT` — 30.07 s and 30.09 s, `view = 2`
        /// both times — for inputs that land in milliseconds.
        ///
        /// Here the confirmations are the late half.
        #[test]
        fn propose_rebuilds_when_the_confirmations_arrive() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 21);
                let committee = seats.members.clone();
                let key = outcome(22);
                let held: Vec<(u8, B256)> = quorum_logs();
                let (mut agree, _bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&held),
                    MockPinned::new(PinnedDerive::Derived(Box::new(key.clone()))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let rx = agree.propose(ctx_for(&committee)).await;
                context.sleep(Duration::from_millis(500)).await;
                assert!(
                    agree.reported().contains(&(TARGET, BAR_QUORUM_UNMET)),
                    "the first attempt must have run and refused before the inputs land"
                );

                seats.seed_pool(&held, seats.bar());
                let digest = settle_digest(&context, rx)
                    .await
                    .expect("a leader must re-read its inputs, not wait out its whole view");
                assert_eq!(digest, seats.proposal(held, key).digest());
            });
        }

        /// The same, with the dealer log as the late half — the edge the confirm
        /// pool does not raise itself, so the beacon actor raises it from
        /// `publish_recorded_logs`. Both inputs must wake the leader or the retry
        /// only covers half the race.
        #[test]
        fn propose_rebuilds_when_a_late_dealer_log_lands() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 23);
                let committee = seats.members.clone();
                let key = outcome(24);
                let held: Vec<(u8, B256)> = quorum_logs();
                let short: Vec<(u8, B256)> = held.iter().copied().take(3).collect();
                let recorded = index_holding(&short);
                seats.seed_pool(&held, seats.bar());
                let (mut agree, _bodies) = agree_over(
                    &context,
                    committee.clone(),
                    recorded.clone(),
                    MockPinned::new(PinnedDerive::Derived(Box::new(key.clone()))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let rx = agree.propose(ctx_for(&committee)).await;
                context.sleep(Duration::from_millis(500)).await;
                assert!(
                    agree.reported().contains(&(TARGET, "quorum_not_met")),
                    "the first attempt must have run and refused before the log lands"
                );

                let (idx, hash) = held[3];
                recorded
                    .write()
                    .expect("index")
                    .entry(TARGET)
                    .or_default()
                    .insert(idx, hash);
                seats.pool.note_inputs_grew();

                let digest = settle_digest(&context, rx)
                    .await
                    .expect("a late dealer log must wake the leader, not the leader timeout");
                assert_eq!(digest, seats.proposal(held, key).digest());
            });
        }

        /// Rider (iv) of the beacon actor's delivery-side mint edge, asserted on the
        /// leader that depends on it. [`ShareConfirm::covers`] is a STRICT superset
        /// test, so peer confirmations minted at a narrower width stop covering the
        /// moment the leader's own set GROWS: growth shrinks the covering count, here
        /// all the way to zero, and the leader refuses until peers re-mint AT the new
        /// width. That is why the actor mints on the path every member runs rather
        /// than only on a leader's — a leader cannot raise its own bar.
        #[test]
        fn a_widened_local_set_refuses_until_peers_reconfirm_at_the_new_width() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 25);
                let committee = seats.members.clone();
                let key = outcome(26);
                let held: Vec<(u8, B256)> = quorum_logs();
                let short: Vec<(u8, B256)> = held.iter().copied().take(3).collect();
                let recorded = index_holding(&short);
                // Peers confirm the NARROW set — the width they held when they minted.
                seats.seed_pool(&short, seats.bar());
                let (mut agree, _bodies) = agree_over(
                    &context,
                    committee.clone(),
                    recorded.clone(),
                    MockPinned::new(PinnedDerive::Derived(Box::new(key.clone()))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let rx = agree.propose(ctx_for(&committee)).await;
                context.sleep(Duration::from_millis(500)).await;
                assert!(
                    agree.reported().contains(&(TARGET, "quorum_not_met")),
                    "the first attempt must have run and refused below the quorum"
                );

                // The late log lands. Local is now AT the quorum — and every
                // confirmation on the wire is one seat too narrow to cover it.
                let (idx, hash) = held[3];
                recorded
                    .write()
                    .expect("index")
                    .entry(TARGET)
                    .or_default()
                    .insert(idx, hash);
                seats.pool.note_inputs_grew();
                context.sleep(Duration::from_millis(500)).await;
                assert!(
                    seats.pool.covering(TARGET, &held).is_empty(),
                    "growth SHRANK the covering count to zero — the narrow confirmations \
                     the leader had are not a superset of the set it now proposes"
                );
                assert!(
                    agree.reported().contains(&(TARGET, BAR_QUORUM_UNMET)),
                    "so the leader refuses at the bar, having cleared the quorum check"
                );

                // Peers re-mint at the new width — on the actor side that is the
                // delivery-path mint this rider exists to justify.
                seats.seed_pool(&held, seats.bar());
                let digest = settle_digest(&context, rx)
                    .await
                    .expect("re-confirmation at the new width must release the leader");
                assert_eq!(digest, seats.proposal(held, key).digest());
            });
        }

        /// Journal replay re-fires every reported activity, so the certificate
        /// leaves at most once.
        ///
        /// And it leaves WITHOUT the body — which is the whole point. On replay
        /// after a restart the in-memory body buffer is empty; a reporter that
        /// resolved the body itself could only peek that buffer (its `report` is
        /// awaited inline by the replay loop), find nothing, and deliver nothing —
        /// leaving the supervisor waiting on a verdict that never comes, the
        /// instance never torn down and its partition never reclaimed.
        #[test]
        fn reporter_delivers_the_certificate_once_and_without_the_body() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|_| async move {
                let (tx, mut rx) = tokio::sync::mpsc::channel(4);
                let mut reporter = DkgReporter::new(TARGET, tx);

                // No body anywhere: this is the post-restart shape.
                let digest = Committee::new(N, 35)
                    .proposal(quorum_logs(), outcome(18))
                    .digest();
                let finalization = finalization_over(digest);
                reporter
                    .report(Activity::Finalization(finalization.clone()))
                    .await;
                reporter.report(Activity::Finalization(finalization)).await;

                let delivered = rx.try_recv().expect("certificate delivered");
                assert_eq!(delivered.proposal.payload, digest);
                assert!(
                    rx.try_recv().is_err(),
                    "the certificate was delivered more than once"
                );
            });
        }

        /// The censorship diff: the logs this node holds that the agreed set
        /// dropped, counted where the body is known.
        #[test]
        fn omission_diff_counts_the_logs_the_agreed_set_dropped() {
            // Locally we also hold seat 4; the agreed set stops at seat 3.
            let mut held = quorum_logs();
            held.push((4, B256::repeat_byte(0x44)));
            let metrics = BeaconMetrics::default();
            let proposal = Committee::new(N, 37).proposal(quorum_logs(), outcome(18));

            note_omissions(&index_holding(&held), TARGET, N, &metrics, &proposal);
            assert_eq!(
                metrics.dkg_agree_logs_omitted.get(),
                1,
                "the one local log the agreed set omitted was not counted"
            );

            note_omissions(
                &index_holding(&quorum_logs()),
                TARGET,
                N,
                &metrics,
                &proposal,
            );
            assert_eq!(
                metrics.dkg_agree_logs_omitted.get(),
                1,
                "an agreed set that dropped nothing must count nothing"
            );
        }

        /// Anything that is not a finalization is not the agreement's output.
        #[test]
        fn reporter_ignores_non_finalization_activity() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|_| async move {
                let (tx, mut rx) = tokio::sync::mpsc::channel(4);
                let mut reporter = DkgReporter::new(TARGET, tx);
                reporter.report(Activity::Nullify(a_nullify())).await;
                assert!(rx.try_recv().is_err());
            });
        }

        /// A context whose parent is a value certified at view 1 — the shape every
        /// view after a certification sees.
        fn ctx_after(
            committee: &[PeerPubkey],
            certified: Digest,
        ) -> SimplexContext<Digest, PeerPubkey> {
            SimplexContext {
                round: commonware_consensus::types::Round::new(
                    Epoch::new(TARGET),
                    commonware_consensus::types::View::new(2),
                ),
                leader: committee[0].clone(),
                parent: (commonware_consensus::types::View::new(1), certified),
            }
        }

        /// A set that is not `quorum_logs()` but is just as acceptable: same key,
        /// one more pinned seat.
        fn wider_logs() -> Vec<(u8, B256)> {
            let mut logs = quorum_logs();
            logs.push((4, B256::repeat_byte(0x4F)));
            logs
        }

        /// The structural bar against two agreed values. Once a value is certified,
        /// the parent simplex hands every later view names it, and a proposal that
        /// names anything else is permanently unacceptable — `simplex` forbids
        /// conflicting finalizations only WITHIN a view, so nothing else stops a
        /// quorum that missed a body at view `v` from certifying a different set at
        /// `v+1` and leaving the epoch with two keys.
        #[test]
        fn verify_refuses_a_value_that_replaces_the_certified_one() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 21);
                let committee = seats.members.clone();
                let key = outcome(22);
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&[]),
                    MockPinned::new(PinnedDerive::Derived(Box::new(key.clone()))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let certified = seats.proposal(quorum_logs(), key.clone());
                let certified_digest = certified.digest();
                let replacement = seats.proposal(wider_logs(), key);
                let replacement_digest = replacement.digest();
                assert_ne!(certified_digest, replacement_digest);
                drop(bodies.broadcast(Recipients::All, certified).await);
                drop(bodies.broadcast(Recipients::All, replacement).await);

                // With nothing certified yet the replacement is a perfectly good
                // proposal, so the refusal below comes from the parent alone.
                let rx = agree.verify(ctx_for(&committee), replacement_digest).await;
                assert_eq!(settle(&context, rx).await, Some(true));

                let rx = agree
                    .verify(ctx_after(&committee, certified_digest), replacement_digest)
                    .await;
                assert_eq!(
                    settle(&context, rx).await,
                    Some(false),
                    "a proposal replacing a certified value must be refused"
                );

                // The certified value itself still verifies, so the bar is not a
                // blanket refusal of every view after a certification.
                let rx = agree
                    .verify(ctx_after(&committee, certified_digest), certified_digest)
                    .await;
                assert_eq!(settle(&context, rx).await, Some(true));
            });
        }

        /// The proposer half of the same bar: a leader elected after a value is
        /// certified re-proposes exactly it, and the relay ships that body.
        #[test]
        fn propose_re_proposes_the_certified_value() {
            let runner = deterministic::Runner::timed(Duration::from_secs(600));
            runner.start(|context| async move {
                let seats = Committee::new(N, 23);
                let committee = seats.members.clone();
                // The set this node would BUILD is a different one, so a re-proposal
                // cannot be confused with a fresh build.
                let (mut agree, bodies) = agree_over(
                    &context,
                    committee.clone(),
                    index_holding(&wider_logs()),
                    MockPinned::new(PinnedDerive::Derived(Box::new(outcome(24)))),
                    RecordingResolver::default(),
                    seats.pool.clone(),
                )
                .await;

                let certified = seats.proposal(quorum_logs(), outcome(25));
                let certified_digest = certified.digest();
                drop(bodies.broadcast(Recipients::All, certified.clone()).await);

                let rx = agree.propose(ctx_after(&committee, certified_digest)).await;
                let digest = tokio::select! {
                    _ = context.sleep(PARK_WINDOW) => panic!("propose parked"),
                    d = rx => d.expect("propose resolved"),
                };
                assert_eq!(
                    digest, certified_digest,
                    "a leader after a certified value must re-propose it, not rebuild"
                );

                agree.broadcast(digest, Plan::Propose).await;
                assert_eq!(
                    bodies.get(digest).await.expect("relay broadcast the body"),
                    certified
                );
            });
        }

        #[test]
        fn genesis_is_constant_per_target_epoch() {
            assert_eq!(agreement_genesis(TARGET), agreement_genesis(TARGET));
            assert_ne!(agreement_genesis(TARGET), agreement_genesis(TARGET + 1));
        }
    }
}
