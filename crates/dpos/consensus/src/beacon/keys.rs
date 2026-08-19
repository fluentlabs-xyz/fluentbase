//! The per-epoch beacon group key `PK_epoch`: one store, one resolution policy.
//!
//! `PK_epoch` is agreed chain data — the group public key of the committee's
//! beacon DKG, minted at a CHANGE-epoch boundary block's `beacon_outcome` and
//! carried forward across the stable epochs that follow. Everything that
//! verifies a wire-received certificate needs it: with a pin
//! `CombinedScheme::verify_certificate` checks the recovered seed, without one it
//! early-returns after the multisig quorum (vote-only admission).
//!
//! Two things live here because they were previously answered twice, differently:
//!
//! - **The store** ([`BeaconKeys`]). Shaped on [`crate::beacon::certify::SeedStore`]:
//!   a newtype so [`BeaconKeys::record`] is the only insertion path, a synchronous
//!   [`BeaconKeys::lookup`] that never blocks and never does I/O (the vote path
//!   calls it without an await), and an `Arc<Notify>` whose permit survives having
//!   no waiter, handed out by [`BeaconKeys::notifier`].
//!
//! - **The ladder** ([`BeaconKeys::get_pk`]). Its ORDER is load-bearing and its
//!   absent-boundary branch has an outage behind it — see each function's docs.
//!
//! ## Why `Notify` and not `watch`
//!
//! A consumer must capture the notifier ONCE, before its loop, and re-arm
//! `notified()` per iteration. That is exactly what `epoch_manager`'s run loop
//! does with the other edges, and it is safe because a `Notify` permit is
//! object-scoped: a [`record`](BeaconKeys::record) landing between iteration N
//! and N+1 is held and consumed by N+1. A `watch` receiver is baselined at the
//! CURRENT version when it is subscribed, so a per-iteration `subscribe()` would
//! silently swallow exactly that fill — reproducing the stuck-consumer bug this
//! edge exists to close.
//!
//! `notify_one` (not `notify_waiters`) for the same reason
//! [`crate::beacon::certify::SeedStore`] uses it: `notify_waiters` stores no
//! permit and re-opens the lost-wakeup window. The waiter population is one; if a
//! genuine second consumer ever appears, hand out a per-consumer notifier then —
//! a single `notify_one` shared by two waiters silently swallows wakes.

use crate::{
    beacon::{
        carry::{chain_key_epoch, DkgQualFor},
        outcome::{group_public_key, parse_outcome, OutcomeError},
        seed::GroupPublic,
    },
    epocher::OriginEpocher,
    order_block::OrderBlock,
    outer::{MarshalMailbox, SCHEME_RETENTION_EPOCHS},
};
use commonware_consensus::types::{Epoch, Epocher as _, Height};
use futures::future::BoxFuture;
use std::{
    collections::BTreeMap,
    num::NonZeroU64,
    sync::{Arc, RwLock},
};
use tokio::sync::Notify;
use tracing::{debug, info, warn};

/// Provenance tier of a [`BeaconKeys`] entry. Ordered: attested outranks
/// local — on a CONFLICTING insert an observed value DISPLACES a local one,
/// never vice-versa (see [`BeaconKeys::record`]). The prior untiered
/// first-write-wins policy let a diverged local W1 write beat the network's
/// W4 observed-outcome write by 1.3 s of timing — trust inverted (soak
/// 2026-07-14, v5@epoch77).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeySource {
    /// This node's OWN DKG material (writers W1/W3 and the ladder-dkg
    /// memoize) — locally reconstructed, can diverge from the chain.
    LocalDkg,
    /// The key in force at an epoch that did NOT re-mint, derived from an
    /// attested key at the minting epoch plus the chain's `dkgQual` bit saying
    /// this epoch carried it forward.
    ///
    /// Stronger than [`Self::LocalDkg`]: both of its inputs are chain facts,
    /// where a local reconstruction can diverge from the chain (soak
    /// 2026-07-14). Weaker than [`Self::ObservedOutcome`]: nobody attested this
    /// key FOR this epoch, which is why [`BeaconKeys::attested`] must keep
    /// ignoring it — that accessor feeds the promote value-gate, and telling it
    /// a carried key was network-attested for an epoch is the one lie it cannot
    /// absorb.
    Carried,
    /// Agreed chain data: a finalized change-epoch boundary block's
    /// `beacon_outcome` (writer W4, and the cert-inlet's BLS-verified boundary
    /// cert) — validated by quorum at vote time.
    ObservedOutcome,
}

/// First 8 serialized bytes of a group public key, hex — a stable, greppable
/// value fingerprint. Enough to byte-diff key VALUES across nodes from logs
/// alone (the reject-triage need); the full G2 hex is 192 chars of log noise.
pub fn pk_prefix(pk: &GroupPublic) -> String {
    let mut s = pk.to_string();
    s.truncate(16);
    s
}

/// The CROSS-EPOCH shared `epoch → PK_epoch` store.
///
/// # The model
///
/// **Producers call [`set_pk`](Self::set_pk) the moment they obtain a key.
/// Consumers call [`get_pk`](Self::get_pk) and think about nothing else.**
/// Everything below is this type's problem, not a caller's: which epoch actually
/// minted the key in force, whether that takes a walk over local blocks or a
/// fetch from a peer, and remembering the answer so the next ask is a map hit.
///
/// The one thing a consumer decides is **what it may spend**, and it says so by
/// passing a [`KeySources`]. The rungs differ by orders of latency — memory,
/// disk, network — and only the caller knows its own budget. The cert-inlet on
/// the vote path passes no `fetch`; the repair sweep, off that path, does.
///
/// Two other reads exist and NEITHER is a cheaper tier of `get_pk`:
///
/// - [`attested`](Self::attested) answers "what did the NETWORK attest for this
///   epoch", not "which key is in force". It is the promote value-gate's input,
///   so a derived value must never appear there — that is what
///   [`KeySource::Carried`] exists to keep out.
/// - [`cached_only`](Self::cached_only) is a raw store probe for the two callers
///   that want the MISS as their answer: one uses it as a trigger to do work, the
///   other as an ordering tripwire. Upgrading either to a resolve would stop the
///   tripwire tripping.
///
/// Durability is a layer on this store, not a second source of truth: see
/// [`crate::beacon::key_journal`], which also decides which entries may survive
/// a restart at all.
///
/// Created ONCE at the launch site and cloned by `Arc` into every consumer: the
/// `FluentApp` clones (hence every per-epoch engine), `epoch_manager`, and both
/// cert-inlets. It must OUTLIVE every engine (engines are aborted at the
/// transition) and be the SAME store for `E` and `E+1` — writer W2 is writer W1
/// of the previous epoch. NEVER build a second one: the writers fill one store
/// and the vote path reads it; two stores silently re-open the boundary forge arm
/// and let a follower's inlet hold `PK_E` while everything else sees nothing.
#[derive(Clone)]
pub struct BeaconKeys {
    map: Arc<RwLock<BTreeMap<u64, (GroupPublic, KeySource)>>>,
    notify: Arc<Notify>,
    /// Durable sink. `None` ⇒ RAM-only, the pre-durability behaviour, which is
    /// what every test and any config without a journal partition gets.
    ///
    /// `UnboundedSender::send` is synchronous and never blocks, which is what
    /// lets the durable write sit inside [`set_pk`](BeaconKeys::set_pk) without
    /// putting the writer behind an await. What in-process readers see is the RAM
    /// map; durability is only ever read by a LATER process, so it is free to
    /// lag. Do NOT swap this for a bounded channel.
    ///
    /// Which entries actually reach the disk is decided by
    /// [`crate::beacon::key_journal`], not here — everything is offered, and the
    /// journal declines what must not survive.
    persist: Option<tokio::sync::mpsc::UnboundedSender<(u64, GroupPublic, KeySource)>>,
}

impl BeaconKeys {
    pub fn new() -> Self {
        Self {
            map: Arc::new(RwLock::new(BTreeMap::new())),
            notify: Arc::new(Notify::new()),
            persist: None,
        }
    }

    /// Join the RAM map to its durable half at startup: seed it with what the
    /// journal held and give it the sink for everything written from now on.
    ///
    /// Restored entries keep the provenance they were WRITTEN under. Defaulting
    /// it either way is a real fault, not a tidiness question: everything
    /// restored as `Carried` disables the carry-divergence guard until a fresh
    /// attested write lands, and everything restored as `ObservedOutcome` makes a
    /// derived value visible to [`Self::attested`], which the promote value-gate
    /// reads.
    pub fn with_persistence(
        rehydrated: Vec<(u64, GroupPublic, KeySource)>,
        persist: tokio::sync::mpsc::UnboundedSender<(u64, GroupPublic, KeySource)>,
    ) -> Self {
        let map = rehydrated
            .into_iter()
            .map(|(epoch, pk, source)| (epoch, (pk, source)))
            .collect();
        Self {
            map: Arc::new(RwLock::new(map)),
            notify: Arc::new(Notify::new()),
            persist: Some(persist),
        }
    }

    /// Whatever is ALREADY recorded for `epoch`, whatever its provenance —
    /// a raw store probe, never a resolve.
    ///
    /// Synchronous, never blocks, never does I/O, never errors; a poisoned lock
    /// degrades to a miss rather than propagating a panic into a hot path. It
    /// exists beside [`Self::get_pk`] for the two callers that want the MISS as
    /// their answer rather than as a reason to go looking: one uses it as a
    /// trigger to do work, the other as an ordering tripwire. Silently upgrading
    /// either to a resolve would stop the tripwire tripping.
    ///
    /// Anything asking "which key is in force at `epoch`" wants [`Self::get_pk`].
    pub fn cached_only(&self, epoch: u64) -> Option<GroupPublic> {
        self.map.read().ok()?.get(&epoch).map(|&(pk, _)| pk)
    }

    /// The NETWORK-ATTESTED `PK_epoch` for `epoch`, if one is known: a
    /// [`KeySource::ObservedOutcome`] entry only. Local-sourced entries are
    /// deliberately invisible here — the promote value-gate must never compare a
    /// local resolve against another local resolve.
    pub fn attested(&self, epoch: u64) -> Option<GroupPublic> {
        self.map.read().ok().and_then(|m| {
            m.get(&epoch)
                .and_then(|&(pk, src)| (src == KeySource::ObservedOutcome).then_some(pk))
        })
    }

    /// The ONLY insertion path — a tiered, idempotent write under the
    /// ATTESTED-SOURCE-WINS conflict policy. An epoch's group key is agreed chain
    /// data; on a DIFFERING re-insert the higher-provenance value holds the entry:
    /// a [`KeySource::ObservedOutcome`] write DISPLACES a differing
    /// [`KeySource::LocalDkg`] one, never vice-versa, and WITHIN a tier the first
    /// write wins. Same-value re-inserts keep the strongest provenance (an
    /// observed confirm upgrades a local entry to attested — the promote
    /// value-gate's input, [`Self::attested`]). Failures are never inserted (the
    /// callers only reach here with a resolved key).
    ///
    /// Fires the `notify` permit UNCONDITIONALLY, including on an idempotent
    /// re-record and on a write the conflict policy discarded: the waiter's job is
    /// to re-run its own resolve, which is idempotent, and a conditional fire
    /// would have to reason about which of the tiering branches can change a
    /// reader's answer.
    pub fn set_pk(&self, epoch: u64, pk: GroupPublic, source: KeySource) {
        self.insert(epoch, pk, source);
        self.notify.notify_one();
        // Durable half, strictly AFTER the notify so wakeup latency is unchanged,
        // and strictly non-blocking so no writer ever parks here.
        if let Some(tx) = self.persist.as_ref() {
            if tx.send((epoch, pk, source)).is_err() {
                warn!(
                    epoch,
                    "key journal writer is gone; key recorded in memory only"
                );
            }
        }
    }

    fn insert(&self, epoch: u64, pk: GroupPublic, source: KeySource) {
        let Ok(mut m) = self.map.write() else {
            warn!(epoch, "beacon key store poisoned; dropping resolved key");
            return;
        };
        let Some(&(existing, existing_src)) = m.get(&epoch) else {
            m.insert(epoch, (pk, source));
            return;
        };
        if existing == pk {
            if source > existing_src {
                m.insert(epoch, (pk, source));
            }
            return;
        }
        // A DIFFERING value for one epoch is the network-wide key-divergence
        // witness (soak 2026-07-14: v5's own W1 value vs the network's ⇒ a lone
        // reject{bad_signature}) — keep it LOUD + counted whichever side wins.
        let winner = match source.cmp(&existing_src) {
            std::cmp::Ordering::Greater => "observed_displaces_local",
            std::cmp::Ordering::Less => "attested_kept",
            std::cmp::Ordering::Equal => "first_write_kept",
        };
        warn!(
            epoch,
            existing = %pk_prefix(&existing),
            existing_source = ?existing_src,
            offered = %pk_prefix(&pk),
            offered_source = ?source,
            winner,
            "group-key re-insert with a DIFFERING value"
        );
        metrics::counter!("dpos_group_key_conflict_total", "winner" => winner).increment(1);
        if source > existing_src {
            m.insert(epoch, (pk, source));
        }
        // Two DIFFERING observed-outcome values would mean two finalized boundary
        // blocks disagree on one epoch's mint — fork-grade, never a handled state.
        debug_assert!(
            !(source == KeySource::ObservedOutcome && existing_src == KeySource::ObservedOutcome),
            "two observed agreed group keys differ for epoch {epoch}"
        );
    }

    /// Drop every entry below `oldest`. Every reader is an exact per-epoch ask for an
    /// epoch near the entered frontier, oldest bounded by the scheme-retention
    /// window, so entries older than that can never be read again — without this
    /// the store grows unbounded across a months-long process.
    pub fn retain_from(&self, oldest: u64) {
        if let Ok(mut m) = self.map.write() {
            m.retain(|e, _| *e >= oldest);
        }
    }

    /// Nothing recorded yet — the "this path memoized no key at all" assertion,
    /// which no per-epoch [`cached_only`](Self::cached_only) can express.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.read().map(|m| m.is_empty()).unwrap_or(false)
    }

    /// A clone of the record-notifier, for a consumer's `select!` arm. Capture it
    /// ONCE, before the loop — see the module docs. `notified()` on the returned
    /// handle consumes any permit stored by a [`record`](Self::record) that fired
    /// before the waiter parked, so a fill landing while nobody is parked is still
    /// seen by the next waiter.
    pub fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }
}

impl Default for BeaconKeys {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode a block's `beacon_outcome` bytes into the `PK_epoch` they assert. The
/// ONE extract: every site that wants "what key does this block mint" goes
/// through here, so the decode config and the group-key projection cannot drift
/// between the propose gate, the verify gate, the inlet and the walk.
pub fn asserted_key(bytes: &[u8]) -> Result<GroupPublic, OutcomeError> {
    parse_outcome(bytes).map(|o| *group_public_key(&o))
}

/// The `PK_epoch` a block mints, or `None` when it mints none (a stable epoch's
/// first block) or the bytes do not decode. For callers that treat "no key here"
/// uniformly; the walk needs the finer [`BoundaryOutcome`] instead.
pub fn minted_key(block: &OrderBlock) -> Option<GroupPublic> {
    asserted_key(block.beacon_outcome.as_ref()?).ok()
}

/// What one rung of the boundary walk found. Four states, not two — collapsing
/// [`Self::Carried`] into a refusal would break the ordinary steady state, and
/// collapsing [`Self::Absent`] into one would re-open the outage described on
/// [`BoundaryWalk::key_for`].
// A `GroupPublic` (G2) is ~288 B; this is a transient return value matched
// immediately by the walk and never stored, so the stack copy is cheaper than
// the per-hop heap allocation boxing would add — the same trade `KeyLookup`
// makes (`application.rs`).
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum BoundaryOutcome {
    /// The boundary block is not in local storage. UNKNOWN — never "no rotation".
    Absent,
    /// Present, carrying no `beacon_outcome`: this epoch did not rotate, so it
    /// provably carried the previous key forward.
    Carried,
    /// Present, carrying a well-formed outcome: this epoch minted `PK_epoch`.
    Minted(GroupPublic),
    /// Present, but the outcome bytes do not decode.
    Unparseable(OutcomeError),
}

/// Classify a by-height boundary read for the walk.
pub fn classify(block: Option<&OrderBlock>) -> BoundaryOutcome {
    let Some(block) = block else {
        return BoundaryOutcome::Absent;
    };
    let Some(bytes) = block.beacon_outcome.as_ref() else {
        return BoundaryOutcome::Carried;
    };
    match asserted_key(bytes) {
        Ok(pk) => BoundaryOutcome::Minted(pk),
        Err(e) => BoundaryOutcome::Unparseable(e),
    }
}

/// By-height read of a boundary block out of the node's OWN marshal archive.
/// A closure rather than a `MarshalMailbox` bound so the walk is unit-testable
/// against a canned archive.
pub type BoundaryBlockAt =
    Arc<dyn Fn(Height) -> BoxFuture<'static, Option<OrderBlock>> + Send + Sync>;

/// This node's OWN DKG material for an epoch, key only (never the share).
pub type OwnKeyFor = Arc<dyn Fn(u64) -> Option<GroupPublic> + Send + Sync>;

/// The walk rung of the ladder, ready to run: a by-height boundary reader plus
/// the geometry that turns an epoch into the height to read. Bundled because the
/// two are useless apart and a caller holding one without the other would have to
/// re-derive `activation + epoch * interval` — the duplicate this module exists
/// to stop.
#[derive(Clone)]
pub struct BoundaryWalk {
    read: BoundaryBlockAt,
    epocher: OriginEpocher,
}

impl BoundaryWalk {
    /// Over the node's own marshal archive. `interval` is `NonZeroU64` because
    /// that is what [`OriginEpocher::new`] takes — reusing the one epoch→height
    /// authority rather than re-deriving it at a second site.
    pub fn over_marshal(marshal: MarshalMailbox, activation: u64, interval: NonZeroU64) -> Self {
        Self::new(
            marshal_boundary_reader(marshal),
            OriginEpocher::new(activation, interval),
        )
    }

    pub fn new(read: BoundaryBlockAt, epocher: OriginEpocher) -> Self {
        Self { read, epocher }
    }

    /// Resolve `PK_epoch` by walking epoch first-blocks BACKWARD from `epoch`
    /// through the marshal's stored blocks. THE
    /// absent/carried/minted/unparseable policy — four branches, and each one is
    /// load-bearing:
    ///
    /// - **absent ⇒ refuse.** Absent is UNKNOWN, not "no rotation": `epoch` itself
    ///   may be the change boundary whose missing block holds the ROTATED key, and
    ///   walking past it pins the STALE pre-rotation key, after which the marshal's
    ///   `verify_delivered` silently rejects (`verified = false`, no log) every
    ///   valid cert of `epoch`. A validator demoted AT a change boundary froze
    ///   exactly this way — its demotion-boundary block was never stored locally.
    ///   This is the branch the cert-inlet's old unbounded carry-forward cursor got
    ///   wrong.
    /// - **minted ⇒ use it.**
    /// - **unparseable ⇒ refuse**, never guess a key from further back.
    /// - **carried ⇒ step back one epoch.** A stable epoch provably carried the key
    ///   forward, so this is the branch that makes an unchanged committee
    ///   resolvable at all; refusing here would break the ordinary steady state.
    ///
    /// Bounded to the scheme-retention window ([`SCHEME_RETENTION_EPOCHS`]) so a
    /// long non-change stretch does not walk unboundedly. Exhausting the bound ⇒
    /// `None` ⇒ vote-only admission, the accepted residual — and the reason
    /// [`BeaconKeys::get_pk`] consults the store and this node's own DKG material first.
    pub async fn key_for(&self, epoch: u64) -> Option<GroupPublic> {
        self.minted_key_for(epoch).await.map(|(_, pk)| pk)
    }

    /// [`Self::key_for`] with the epoch the key was MINTED at, which the walk
    /// knows (it is where it stopped stepping back) and used to discard.
    ///
    /// The caller needs it to file the key truthfully: a key found by walking
    /// back from `epoch` was attested for the MINTING epoch, not for `epoch`.
    pub async fn minted_key_for(&self, epoch: u64) -> Option<(u64, GroupPublic)> {
        let mut e = epoch;
        for _ in 0..SCHEME_RETENTION_EPOCHS {
            let first = self.epocher.first(Epoch::new(e))?;
            match classify((self.read)(first).await.as_ref()) {
                BoundaryOutcome::Minted(pk) => return Some((e, pk)),
                BoundaryOutcome::Absent => {
                    debug!(
                        epoch,
                        walk = e,
                        "beacon-key walk hit an ABSENT boundary block — vote-only admission \
                         (walking past it could pin a stale pre-rotation key)"
                    );
                    return None;
                }
                BoundaryOutcome::Unparseable(err) => {
                    warn!(
                        epoch,
                        walk = e,
                        ?err,
                        "beacon outcome present but unparseable; vote-only admission"
                    );
                    return None;
                }
                BoundaryOutcome::Carried => e = e.checked_sub(1)?,
            }
        }
        None
    }
}

/// Build the [`BoundaryBlockAt`] over a marshal mailbox.
pub fn marshal_boundary_reader(marshal: MarshalMailbox) -> BoundaryBlockAt {
    Arc::new(move |height: Height| {
        let marshal = marshal.clone();
        Box::pin(async move { marshal.get_block(height).await })
            as BoxFuture<'static, Option<OrderBlock>>
    })
}

/// By-height fetch of a boundary block from a PEER. A closure, like
/// [`BoundaryBlockAt`], so this module stays free of the upstream/cert-follow
/// types: the caller wraps whatever authenticated seam it has (and whatever hash
/// it authenticates the committee at) and hands back a plain block.
pub type BoundaryBlockFetch =
    Arc<dyn Fn(Height) -> BoxFuture<'static, Option<OrderBlock>> + Send + Sync>;

/// The FETCH rung, ready to run — the network sibling of [`BoundaryWalk`].
///
/// Separate from the walk because it answers a question the walk cannot: the
/// walk steps back one epoch per hop over blocks already on disk, so on a
/// committee that has been stable longer than its bound it never reaches the
/// minting block, and on a node that holds none of those blocks it refuses at
/// the first absent one. This asks for exactly ONE height, named by the chain.
#[derive(Clone)]
pub struct BoundaryFetch {
    fetch: BoundaryBlockFetch,
    dkg_qual: DkgQualFor,
    epocher: OriginEpocher,
}

impl BoundaryFetch {
    pub fn new(fetch: BoundaryBlockFetch, dkg_qual: DkgQualFor, epocher: OriginEpocher) -> Self {
        Self {
            fetch,
            dkg_qual,
            epocher,
        }
    }

    /// `(minting epoch, key)` for the key in force at `epoch`, fetched from a
    /// peer. The minting epoch comes back with it because it, not `epoch`, is
    /// what the certificate accompanying that block attests — the caller files
    /// it under that.
    pub async fn key_for(&self, epoch: u64) -> Option<(u64, GroupPublic)> {
        let (minted_at, height) = self.minting_height(epoch)?;
        let block = (self.fetch)(height).await?;
        match classify(Some(&block)) {
            BoundaryOutcome::Minted(pk) => {
                info!(
                    epoch,
                    minted_at,
                    height = height.get(),
                    "resolved PK_epoch from a peer's minting boundary block"
                );
                Some((minted_at, pk))
            }
            // We asked for a height the chain named as a MINT. Anything else
            // means the chain read and the block disagree, and with two
            // disagreeing sources, believing either is a guess.
            other => {
                warn!(
                    epoch,
                    minted_at,
                    height = height.get(),
                    outcome = ?other,
                    "peer served the named minting boundary but it carries no usable key — \
                     staying vote-only rather than guessing"
                );
                None
            }
        }
    }

    /// The epoch that minted the key in force at `epoch`, and that epoch's first
    /// block.
    ///
    /// `None` has two causes and the caller must treat both as "not now", never
    /// as "fetch something else": the `dkgQual` read is UNDECIDED (the outer
    /// `None` of [`chain_key_epoch`]) — a node with no finalized marker yet reads
    /// exactly that — or the epoch predates the beacon entirely.
    fn minting_height(&self, epoch: u64) -> Option<(u64, Height)> {
        let minted_at = chain_key_epoch(epoch, &self.dkg_qual)??;
        Some((minted_at, self.epocher.first(Epoch::new(minted_at))?))
    }
}

/// Which rungs a caller may spend on a [`BeaconKeys::get_pk`]. The store rung is
/// unconditional; everything else is opt-in, because the rungs differ by ORDERS
/// of latency and the caller is the only one who knows its budget.
///
/// Absent rung ⇒ that source is simply not consulted. Never pass `fetch` from a
/// caller on the vote path: its budget is seconds against that path's one.
#[derive(Default, Clone, Copy)]
pub struct KeySources<'a> {
    /// This node's own DKG material. In-memory.
    pub own: Option<&'a OwnKeyFor>,
    /// Bounded backward walk over locally stored boundary blocks. Disk.
    pub walk: Option<&'a BoundaryWalk>,
    /// One authenticated by-height fetch from a peer. Network.
    pub fetch: Option<&'a BoundaryFetch>,
}

/// The ONE `PK_epoch` ladder, and its order is load-bearing:
///
/// 1. **the shared store** — W1 (own key, published before the engine spawns),
///    W4 (an observed boundary outcome) and the inlet's verified boundary certs
///    all land here;
/// 2. **this node's own DKG material**, key only;
/// 3. **the bounded backward marshal walk** ([`BoundaryWalk::key_for`]).
///
/// Ladder-first, walk-last, because BOTH earlier rungs resolve for a committee
/// that has been stable longer than the walk's bound — where the walk goes empty
/// even with every block on disk. Inverting the order would leave such an epoch
/// unpinned and vote-only for no reason.
///
/// `own` is `None` where the caller structurally has no DKG material to consult
/// (a follower) or cannot reach it (the cert-inlet, which sits outside the beacon
/// plane): on a validator W1 publishes that same key into the store BEFORE the
/// engine spawns, so rung 1 already covers it there and the missing rung costs
/// nothing. `walk` is `None` only where there is no marshal to walk (unit tests);
/// the store rung still answers.
impl BeaconKeys {
    /// The key in force at `epoch`, spending only the rungs `sources` permits.
    ///
    /// The ONE way to ask that question. [`Self::cached_only`] and
    /// [`Self::attested`] remain, and neither is a cheaper tier of this: the
    /// first answers "is anything recorded yet", the second "what did the
    /// NETWORK attest for this epoch". Only this one resolves.
    pub async fn get_pk(&self, epoch: u64, sources: KeySources<'_>) -> Option<GroupPublic> {
        if let Some(pk) = self.cached_only(epoch) {
            return Some(pk);
        }
        if let Some(pk) = sources.own.and_then(|f| f(epoch)) {
            return Some(pk);
        }
        if let Some(walk) = sources.walk {
            if let Some((minted_at, pk)) = walk.minted_key_for(epoch).await {
                self.memoise_carry(epoch, minted_at, pk);
                return Some(pk);
            }
        }
        if let Some(fetch) = sources.fetch {
            if let Some((minted_at, pk)) = fetch.key_for(epoch).await {
                // Filed under the epoch that MINTED it — what the certificate
                // accompanying that block attests. Filing it under `epoch` would
                // make `attested(epoch)` report a carried key as directly
                // network-attested for an epoch nobody attested it for.
                self.set_pk(minted_at, pk, KeySource::ObservedOutcome);
                self.memoise_carry(epoch, minted_at, pk);
                return Some(pk);
            }
        }
        None
    }

    /// Record that `epoch` carries the key minted at `minted_at`, so the next
    /// [`Self::get_pk`] for it is a map hit rather than another walk or fetch.
    ///
    /// A no-op when `epoch` IS the minting epoch — the truthful entry is already
    /// there under its own provenance, and writing a weaker one over it would be
    /// the only way this method could lose information.
    ///
    /// [`KeySource::Carried`] and never `ObservedOutcome`: nobody attested this
    /// key FOR this epoch, and [`Self::attested`] is the promote value-gate's
    /// input.
    fn memoise_carry(&self, epoch: u64, minted_at: u64, pk: GroupPublic) {
        if epoch != minted_at {
            self.set_pk(epoch, pk, KeySource::Carried);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::outcome::DkgOutcome;
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1, NZU64};
    use fluentbase_bls::PeerPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::Mutex;

    /// Two distinct group keys, deterministically. `test_outcome`'s key is the
    /// third distinct value; nothing here needs them to be related.
    fn pk(seed: u64) -> GroupPublic {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, _) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        *group_public_key(&outcome)
    }

    /// A real 4-player DKG outcome — the only way to get bytes `parse_outcome`
    /// accepts, and its group key is what the walk must return.
    fn test_outcome() -> DkgOutcome {
        let mut rng = StdRng::seed_from_u64(7);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
            .expect("deal")
            .0
    }

    /// A fetch rung over a canned peer: records every height it was asked for, so
    /// "which height did it choose" and "did it ask at all" are both observable.
    fn fetch_rung(
        served: BTreeMap<u64, OrderBlock>,
        bits: &[u64],
        asked: Arc<std::sync::Mutex<Vec<u64>>>,
    ) -> BoundaryFetch {
        let set: std::collections::BTreeSet<u64> = bits.iter().copied().collect();
        let dkg_qual: DkgQualFor = Arc::new(move |e| Some(set.contains(&e)));
        BoundaryFetch::new(
            Arc::new(move |h: Height| {
                asked.lock().unwrap().push(h.get());
                let block = served.get(&h.get()).cloned();
                Box::pin(async move { block }) as BoxFuture<'static, Option<OrderBlock>>
            }),
            dkg_qual,
            OriginEpocher::new(0, std::num::NonZeroU64::new(10).unwrap()),
        )
    }

    /// The memo, and what it is for: the SECOND ask for a carried epoch must not
    /// pay for the walk again. Without it every `get_pk` on a long-stable
    /// committee re-walks the archive, which is the cost the owner's
    /// set-then-get model exists to remove.
    ///
    /// Reds if `memoise_carry` stops writing.
    #[tokio::test]
    async fn a_carried_epoch_is_walked_once_and_then_hit_in_the_map() {
        let outcome = test_outcome();
        let expected = *group_public_key(&outcome);
        // 40 and 39 carry; 38 mints.
        let (read, asked) = reader(vec![
            (400, None),
            (390, None),
            (380, Some(crate::beacon::outcome::encode_outcome(&outcome))),
        ]);
        let walk = walk(read);
        let store = BeaconKeys::new();
        let sources = KeySources {
            walk: Some(&walk),
            ..Default::default()
        };

        assert_eq!(store.get_pk(40, sources).await, Some(expected));
        let after_first = asked.lock().unwrap().len();
        assert!(after_first > 1, "the first ask really walked");

        assert_eq!(store.get_pk(40, sources).await, Some(expected));
        assert_eq!(
            asked.lock().unwrap().len(),
            after_first,
            "the second ask must not touch the archive at all"
        );
    }

    /// The memo is filed as CARRIED, so it is readable as the key in force and
    /// invisible to `attested`, and the minting epoch keeps its own truthful
    /// entry. Filing the carry as observed would tell the promote value-gate the
    /// network attested this key for an epoch it never did.
    ///
    /// Reds if the memo's source changes, and reds if it is written under the
    /// minting epoch instead of the carrying one.
    #[tokio::test]
    async fn the_memo_is_carried_and_leaves_the_mint_entry_alone() {
        let outcome = test_outcome();
        let expected = *group_public_key(&outcome);
        let (read, _) = reader(vec![
            (400, None),
            (390, Some(crate::beacon::outcome::encode_outcome(&outcome))),
        ]);
        let walk = walk(read);
        let store = BeaconKeys::new();

        store
            .get_pk(
                40,
                KeySources {
                    walk: Some(&walk),
                    ..Default::default()
                },
            )
            .await;

        assert_eq!(store.cached_only(40), Some(expected), "readable at 40");
        assert_eq!(store.attested(40), None, "but never as an attestation");
    }

    /// An undecided `dkgQual` read must produce NO fetch at all. A node with no
    /// finalized marker yet reads exactly that, and aiming at a guessed height
    /// would spend the one authenticated request on the wrong block.
    ///
    /// Reds if the undecided arm falls through to a height instead of returning.
    #[tokio::test]
    async fn an_undecided_chain_read_issues_no_fetch() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let unreadable: DkgQualFor = Arc::new(|_| None);
        let rung = BoundaryFetch::new(
            {
                let asked = asked.clone();
                Arc::new(move |h: Height| {
                    asked.lock().unwrap().push(h.get());
                    Box::pin(async { None }) as BoxFuture<'static, Option<OrderBlock>>
                })
            },
            unreadable,
            OriginEpocher::new(0, std::num::NonZeroU64::new(10).unwrap()),
        );

        assert_eq!(rung.key_for(12).await, None);
        assert!(
            asked.lock().unwrap().is_empty(),
            "an undecided read must not spend the request"
        );
    }

    /// A committee stable since genesis is the devnet default, and it is exactly
    /// where the naive target is wrong: E's own first block carries no outcome.
    /// The height must be the BOOTSTRAP mint's, and the key must come back filed
    /// under that epoch — not under the epoch being repaired, or
    /// [`BeaconKeys::attested`] would claim an attestation nobody made for it.
    ///
    /// Reds if the fetch targets `first(E)`, and reds if the recording moves to
    /// the queried epoch.
    #[tokio::test]
    async fn a_stable_committee_fetches_the_bootstrap_boundary_and_files_it_there() {
        let bootstrap = crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
        let outcome = test_outcome();
        let expected = *group_public_key(&outcome);
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let served = BTreeMap::from([(
            bootstrap * 10,
            block_at(
                bootstrap * 10,
                Some(crate::beacon::outcome::encode_outcome(&outcome)),
            ),
        )]);

        let rung = fetch_rung(served, &[], asked.clone());
        let store = BeaconKeys::new();
        let got = store
            .get_pk(
                12,
                KeySources {
                    fetch: Some(&rung),
                    ..Default::default()
                },
            )
            .await;

        assert_eq!(got, Some(expected));
        assert_eq!(
            *asked.lock().unwrap(),
            vec![bootstrap * 10],
            "the bootstrap mint's first block, not epoch 12's"
        );
        assert_eq!(
            store.cached_only(bootstrap),
            Some(expected),
            "the attestation is filed under the epoch it was made for"
        );
        assert_eq!(
            store.attested(bootstrap),
            Some(expected),
            "and there it IS an attestation"
        );
        assert_eq!(
            store.cached_only(12),
            Some(expected),
            "12 gets the carry memo, so the next ask is a map hit"
        );
        assert_eq!(
            store.attested(12),
            None,
            "but nobody attested this key FOR 12, and the promote value-gate \
             must never be told otherwise"
        );
    }

    /// We asked for a height the chain named as a MINT. A block with no outcome
    /// means the chain read and the block disagree, and with two disagreeing
    /// sources believing either is a guess.
    ///
    /// Reds if the non-`Minted` arm starts returning a key.
    #[tokio::test]
    async fn a_fetched_boundary_carrying_no_outcome_yields_nothing() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let served = BTreeMap::from([(90, block_at(90, None))]);
        let rung = fetch_rung(served, &[9], asked.clone());
        let store = BeaconKeys::new();

        let got = store
            .get_pk(
                12,
                KeySources {
                    fetch: Some(&rung),
                    ..Default::default()
                },
            )
            .await;

        assert_eq!(got, None);
        assert_eq!(*asked.lock().unwrap(), vec![90], "it did ask");
        assert!(
            store.cached_only(9).is_none(),
            "and a refusal leaves nothing half-written"
        );
    }

    /// The soak-2026-07-14 inversion, fixed: a stale LOCAL W1 write landed
    /// first, then the network's W4 observed-outcome key arrived and was
    /// DROPPED by first-write-wins — the poisoned entry then failed the next
    /// epoch's parent-seed witness. Attested-source-wins: the observed
    /// (agreed-chain-data) write must DISPLACE the differing local one.
    #[test]
    fn observed_outcome_displaces_a_differing_local_write() {
        let (local, network) = (pk(1), pk(2));
        assert_ne!(local, network);
        let map = BeaconKeys::new();
        map.set_pk(77, local, KeySource::LocalDkg); // W1 (stale)
        map.set_pk(77, network, KeySource::ObservedOutcome); // W4
        assert_eq!(
            map.cached_only(77),
            Some(network),
            "agreed chain data must beat local reconstruction regardless of timing"
        );
        assert_eq!(map.attested(77), Some(network));
    }

    /// The carried tier sits between the other two, and both directions matter.
    /// A carry is derived from two CHAIN facts, so it must beat a local
    /// reconstruction that soak 2026-07-14 proved can diverge; and nobody
    /// attested it FOR this epoch, so an observed write must still beat it.
    ///
    /// Reds if `Carried` is declared anywhere other than between `LocalDkg` and
    /// `ObservedOutcome` — the conflict policy reads strength through `Ord`, so
    /// the variant's POSITION is the policy.
    #[test]
    fn carried_beats_local_and_loses_to_observed() {
        let (local, carried, network) = (pk(1), pk(2), pk(3));
        let map = BeaconKeys::new();

        map.set_pk(77, local, KeySource::LocalDkg);
        map.set_pk(77, carried, KeySource::Carried);
        assert_eq!(
            map.cached_only(77),
            Some(carried),
            "a carry is two chain facts; a local reconstruction is neither"
        );

        map.set_pk(77, network, KeySource::ObservedOutcome);
        assert_eq!(
            map.cached_only(77),
            Some(network),
            "and the epoch's own attestation beats a key carried into it"
        );

        // The reverse direction, on a fresh entry: an observed key already held
        // is not displaced by a carry.
        let map = BeaconKeys::new();
        map.set_pk(77, network, KeySource::ObservedOutcome);
        map.set_pk(77, carried, KeySource::Carried);
        assert_eq!(map.cached_only(77), Some(network));
    }

    /// `attested` is the promote value-gate's input, and the gate compares a
    /// LOCAL resolve against what the NETWORK attested. A carried key is not
    /// that: it was attested for the minting epoch, not for this one. If it
    /// became visible here the gate would compare a derived value against
    /// itself and pass anything.
    ///
    /// Reds if `attested` starts accepting the new variant.
    #[test]
    fn a_carried_entry_is_invisible_to_attested() {
        let map = BeaconKeys::new();
        map.set_pk(77, pk(2), KeySource::Carried);

        assert_eq!(
            map.cached_only(77),
            Some(pk(2)),
            "readable as the key in force"
        );
        assert_eq!(map.attested(77), None, "but never as a network attestation");
    }

    /// The inverse ordering: once an observed key holds the entry, no later
    /// local resolve may displace it (last-write-wins would re-open the
    /// boundary-forge arm the insert-only store exists to close).
    #[test]
    fn local_write_never_displaces_an_attested_entry() {
        let (local, network) = (pk(1), pk(2));
        let map = BeaconKeys::new();
        map.set_pk(77, network, KeySource::ObservedOutcome);
        map.set_pk(77, local, KeySource::LocalDkg);
        assert_eq!(map.cached_only(77), Some(network));
        assert_eq!(map.attested(77), Some(network));
    }

    /// Same value, stronger provenance: an observed confirm UPGRADES a local
    /// entry to attested (the promote value-gate's input); within a tier the
    /// first write wins and the store stays insert-only.
    #[test]
    fn same_value_reinsert_upgrades_provenance_only() {
        let key = pk(1);
        let map = BeaconKeys::new();
        map.set_pk(9, key, KeySource::LocalDkg);
        assert_eq!(
            map.attested(9),
            None,
            "a local-only entry is NOT network-attested"
        );
        map.set_pk(9, key, KeySource::ObservedOutcome);
        assert_eq!(map.attested(9), Some(key));
        // And it never downgrades back.
        map.set_pk(9, key, KeySource::LocalDkg);
        assert_eq!(map.attested(9), Some(key));
    }

    /// The promote value-gate compares ONLY against network-attested entries:
    /// a differing local entry (our own earlier write — possibly the same
    /// stale source) must not masquerade as a network observation.
    #[test]
    fn attested_is_blind_to_local_entries() {
        let key = pk(1);
        let map = BeaconKeys::new();
        map.set_pk(5, key, KeySource::LocalDkg);
        assert_eq!(map.attested(5), None);
        assert_eq!(map.attested(6), None);
    }

    #[tokio::test]
    async fn a_fill_landing_with_no_waiter_is_seen_by_the_next_waiter() {
        let store = BeaconKeys::new();
        let notify = store.notifier();
        store.set_pk(1, pk(1), KeySource::LocalDkg);
        tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
            .await
            .expect("the permit stored with no waiter parked must wake the next waiter");
    }

    #[tokio::test]
    async fn an_idempotent_re_record_still_fires_the_permit() {
        let store = BeaconKeys::new();
        let notify = store.notifier();
        store.set_pk(1, pk(1), KeySource::LocalDkg);
        // Bounded: a regression to `notify_waiters` (no stored permit) must make
        // this test FAIL, not hang the suite.
        tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
            .await
            .expect("the first record's permit");
        store.set_pk(1, pk(1), KeySource::LocalDkg);
        tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
            .await
            .expect("record fires unconditionally, re-record included");
    }

    /// origin 0, length 10 ⇒ first(E) = 10·E.
    fn walk(read: BoundaryBlockAt) -> BoundaryWalk {
        BoundaryWalk::new(read, OriginEpocher::new(0, NZU64!(10)))
    }

    fn block_at(height: u64, outcome: Option<Vec<u8>>) -> OrderBlock {
        OrderBlock {
            parent: crate::digest::Digest(alloy_primitives::B256::ZERO),
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            fee_recipient: alloy_primitives::Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: alloy_primitives::Bytes::new(),
            result: alloy_primitives::B256::ZERO,
            txs: Vec::new(),
            beacon_outcome: outcome.map(alloy_primitives::Bytes::from),
            dkg_logs: Vec::new(),
            parent_seed: None,
            equivocation: None,
        }
    }

    /// Reader over a canned archive; records the heights it was asked for so the
    /// walk's step count is observable.
    fn reader(blocks: Vec<(u64, Option<Vec<u8>>)>) -> (BoundaryBlockAt, Arc<Mutex<Vec<u64>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        let map: BTreeMap<u64, Option<Vec<u8>>> = blocks.into_iter().collect();
        let read: BoundaryBlockAt = Arc::new(move |h: Height| {
            seen.lock().unwrap().push(h.get());
            let hit = map.get(&h.get()).cloned();
            Box::pin(async move { hit.map(|o| block_at(0, o)) })
                as BoxFuture<'static, Option<OrderBlock>>
        });
        (read, asked)
    }

    #[tokio::test]
    async fn the_walk_steps_back_through_carried_epochs_to_the_mint() {
        let outcome = crate::beacon::outcome::encode_outcome(&test_outcome());
        let want = asserted_key(&outcome).unwrap();
        let (read, asked) = reader(vec![(50, None), (40, None), (30, Some(outcome))]);
        assert_eq!(
            walk(read).key_for(5).await,
            Some(want),
            "a stable stretch must resolve from the epoch that minted"
        );
        assert_eq!(*asked.lock().unwrap(), vec![50, 40, 30]);
    }

    #[tokio::test]
    async fn the_walk_refuses_at_an_absent_boundary_and_never_walks_past_it() {
        let outcome = crate::beacon::outcome::encode_outcome(&test_outcome());
        // Epoch 4's boundary is missing; epoch 3's holds a key. Pinning that key
        // for epoch 5 is exactly the stale-pre-rotation freeze.
        let (read, asked) = reader(vec![(50, None), (30, Some(outcome))]);
        assert_eq!(walk(read).key_for(5).await, None);
        assert_eq!(*asked.lock().unwrap(), vec![50, 40]);
    }

    #[tokio::test]
    async fn the_walk_refuses_an_unparseable_outcome_rather_than_guessing_from_further_back() {
        let outcome = crate::beacon::outcome::encode_outcome(&test_outcome());
        let (read, asked) = reader(vec![(50, Some(vec![0xff; 8])), (40, Some(outcome))]);
        assert_eq!(walk(read).key_for(5).await, None);
        assert_eq!(*asked.lock().unwrap(), vec![50]);
    }

    #[tokio::test]
    async fn the_walk_is_bounded_by_the_scheme_retention_window() {
        let carried: Vec<(u64, Option<Vec<u8>>)> = (0..=40u64).map(|e| (e * 10, None)).collect();
        let (read, asked) = reader(carried);
        assert_eq!(walk(read).key_for(40).await, None);
        assert_eq!(asked.lock().unwrap().len(), SCHEME_RETENTION_EPOCHS);
    }

    #[tokio::test]
    async fn the_ladder_prefers_the_store_over_a_walk_that_would_go_empty() {
        let store = BeaconKeys::new();
        store.set_pk(40, pk(9), KeySource::LocalDkg);
        // Every boundary in the window carries nothing: the walk alone is empty.
        let (read, _) = reader((0..=40u64).map(|e| (e * 10, None)).collect());
        assert_eq!(
            store
                .get_pk(
                    40,
                    KeySources {
                        walk: Some(&walk(read)),
                        ..Default::default()
                    }
                )
                .await,
            Some(pk(9))
        );
    }

    #[tokio::test]
    async fn the_ladder_falls_through_to_own_dkg_material_then_to_the_walk() {
        let store = BeaconKeys::new();
        let own: OwnKeyFor = Arc::new(|e| (e == 40).then(|| pk(9)));
        let (read, asked) = reader((0..=40u64).map(|e| (e * 10, None)).collect());
        assert_eq!(
            store
                .get_pk(
                    40,
                    KeySources {
                        own: Some(&own),
                        walk: Some(&walk(read)),
                        ..Default::default()
                    }
                )
                .await,
            Some(pk(9))
        );
        assert!(
            asked.lock().unwrap().is_empty(),
            "an own-DKG hit must not cost a marshal round-trip"
        );

        let outcome = crate::beacon::outcome::encode_outcome(&test_outcome());
        let want = asserted_key(&outcome).unwrap();
        let (read, _) = reader(vec![(400, Some(outcome))]);
        let no_own: OwnKeyFor = Arc::new(|_| None);
        assert_eq!(
            store
                .get_pk(
                    40,
                    KeySources {
                        own: Some(&no_own),
                        walk: Some(&walk(read)),
                        ..Default::default()
                    }
                )
                .await,
            Some(want)
        );
    }
}
