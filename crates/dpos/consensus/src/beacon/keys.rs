//! The per-epoch beacon group key `PK_epoch`: one store, one resolution policy.
//!
//! `PK_epoch` is the group public key of the committee's beacon DKG, minted at a
//! CHANGE epoch by the p2p key-agreement plane and carried forward across the
//! stable epochs that follow. Everything that verifies a wire-received
//! certificate needs it: with a pin `CombinedScheme::verify_certificate` checks
//! the recovered seed, without one it early-returns after the multisig quorum
//! (vote-only admission).
//!
//! No block carries it. The minting epoch's key lives in the agreement plane's
//! quorum-signed artifact, so both non-store rungs below
//! ([`AgreedKeys`]) ask the same question of the same object — locally held, then
//! pulled from a peer — and the chain's own `dkgQual` record names WHICH epoch
//! minted the key in force at the epoch being asked about.
//!
//! Two things live here because they were previously answered twice, differently:
//!
//! - **The store** ([`BeaconKeys`]). Shaped on [`crate::beacon::certify::SeedStore`]:
//!   a newtype so [`BeaconKeys::set_pk`] is the only insertion path, a synchronous
//!   [`BeaconKeys::cached_only`] that never blocks and never does I/O (the vote
//!   path calls it without an await), and an `Arc<Notify>` whose permit survives
//!   having no waiter, handed out by [`BeaconKeys::notifier`].
//!
//! - **The ladder** ([`BeaconKeys::get_pk`]). Its ORDER is load-bearing — see
//!   each function's docs.
//!
//! ## Why `Notify` and not `watch`
//!
//! A consumer must capture the notifier ONCE, before its loop, and re-arm
//! `notified()` per iteration. That is exactly what `epoch_manager`'s run loop
//! does with the other edges, and it is safe because a `Notify` permit is
//! object-scoped: a [`set_pk`](BeaconKeys::set_pk) landing between iteration N
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

use crate::beacon::carry::{chain_key_epoch_memoised, DkgQualFor};
use fluentbase_bls::beacon::GroupPublic;
use futures::future::BoxFuture;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use tokio::sync::Notify;
use tracing::{debug, warn};

/// Provenance tier of a [`BeaconKeys`] entry. Ordered: attested outranks
/// local — on a CONFLICTING insert an observed value DISPLACES a local one,
/// never vice-versa (see [`BeaconKeys::set_pk`]). The prior untiered
/// first-write-wins policy let a diverged local W1 write beat the network's
/// W4 observed-outcome write by 1.3 s of timing — trust inverted (soak
/// 2026-07-14, v5@epoch77).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeySource {
    /// This node's OWN DKG material (writers W1/W3) — locally
    /// reconstructed, can diverge from the chain.
    LocalDkg,
    /// The key in force at an epoch that did NOT re-mint, derived from an
    /// attested key at the minting epoch plus the chain's `dkgQual` bit saying
    /// this epoch carried it forward.
    ///
    /// Stronger than [`Self::LocalDkg`]: both of its inputs are chain facts,
    /// where a local reconstruction can diverge from the chain (soak
    /// 2026-07-14). Weaker than [`Self::Agreed`]: nobody attested this key FOR
    /// this epoch, which is why [`BeaconKeys::attested`] must keep ignoring it —
    /// that accessor feeds the promote value-gate, and telling it a carried key
    /// was network-attested for an epoch is the one lie it cannot absorb.
    Carried,
    /// The epoch-key agreement plane's artifact: a `committee[epoch]` quorum
    /// certified this exact `(epoch, PK_epoch)` under the agreement namespace
    /// ([`crate::beacon::artifact::verify_artifact`]).
    ///
    /// The strongest tier, and the only one nothing about is derived — which is
    /// why it is what [`BeaconKeys::attested`] answers with. It replaced an
    /// `ObservedOutcome` tier that read the value out of a finalized boundary
    /// block; that block no longer carries one.
    Agreed,
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
/// disk, network — and only the caller knows its own budget. Callers on the vote
/// path pass no [`pull`](KeySources::pull) — the cert-inlet, and a follower's
/// `ensure_key` at BOTH efforts; callers off it do — the repair sweep, and the
/// follower's background fetch task.
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
    /// attested write lands, and everything restored as `Agreed` makes a derived
    /// value visible to [`Self::attested`], which the promote value-gate reads.
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

    /// [`Self::cached_only`] with a provenance FLOOR: the recorded entry only if
    /// it was written at `min` or better, otherwise a miss.
    ///
    /// The store mixes tiers — [`KeySource::LocalDkg`] is a value this node
    /// reconstructed and can diverge from the chain (soak 2026-07-14) — and a
    /// caller whose action on the answer is IRREVERSIBLE cannot take that tier.
    /// The repair sweep is that caller: its pin is write-once per epoch, so a
    /// wrong one makes the marshal reject every valid certificate of the epoch
    /// for the life of the process. Refusing the weak tier there is not the same
    /// as skipping the store: the tiers above it are exactly the answers the
    /// sweep wants, and they are reached without re-walking or re-fetching.
    pub fn cached_at_least(&self, epoch: u64, min: KeySource) -> Option<GroupPublic> {
        self.map
            .read()
            .ok()?
            .get(&epoch)
            .and_then(|&(pk, src)| (src >= min).then_some(pk))
    }

    /// The NETWORK-ATTESTED `PK_epoch` for `epoch`, if one is known: a
    /// [`KeySource::Agreed`] entry only — a value a `committee[epoch]` quorum
    /// signed FOR this epoch, not one this node reconstructed or carried
    /// forward. Locally-sourced and carried entries are deliberately invisible
    /// here.
    ///
    /// Two callers, one question. The promote value-gate compares its locally
    /// resolved key against this and demotes on a mismatch; W1 uses the same
    /// answer to stand down, because a local reconstruction has nothing to add
    /// to an epoch a quorum already spoke for. Both would be wrong to see a
    /// local resolve compared against another local resolve.
    pub fn attested(&self, epoch: u64) -> Option<GroupPublic> {
        self.map.read().ok().and_then(|m| {
            m.get(&epoch)
                .and_then(|&(pk, src)| (src == KeySource::Agreed).then_some(pk))
        })
    }

    /// The ONLY insertion path — a tiered, idempotent write under the
    /// ATTESTED-SOURCE-WINS conflict policy. An epoch's group key is agreed chain
    /// data; on a DIFFERING re-insert the higher-provenance value holds the entry:
    /// a [`KeySource::Agreed`] write DISPLACES a differing
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
        // Two DIFFERING agreed values would mean two `committee[epoch]` quorums
        // certified different keys for one epoch — fork-grade, never a handled
        // state.
        debug_assert!(
            !(source == KeySource::Agreed && existing_src == KeySource::Agreed),
            "two quorum-agreed group keys differ for epoch {epoch}"
        );
    }

    /// Drop every DERIVED entry below `oldest`. [`KeySource::Agreed`] entries are
    /// kept for the life of the process, whatever their epoch.
    ///
    /// The window is what bounds the derived tiers, and only they need it:
    /// [`KeySource::LocalDkg`] (a W1/W3 publication) and [`KeySource::Carried`]
    /// (a [`memoise_carry`](Self::memoise_carry) memo) are written once per epoch
    /// ENTERED, so without a trailing window they grow unbounded across a
    /// months-long process, and every reader of them asks for an epoch near the
    /// entered frontier.
    ///
    /// `Agreed` is the opposite shape, and must not take the same window for the
    /// reason [`crate::beacon::key_journal`] already states for the durable half
    /// of this same map: it is written once per COMMITTEE CHANGE and filed under
    /// the epoch that MINTED the key, so on a long-stable committee the entry
    /// worth having is the OLDEST one. Both carry-divergence tripwires
    /// ([`crate::dpos::group_key_resolver`] on the vote path,
    /// [`crate::dpos::beacon_share_resolver`] on the share gate) ask
    /// [`Self::attested`] for the MINTING epoch, which on a committee that never
    /// changes is the bootstrap mint forever — an epoch-measured window disarms
    /// them once the frontier passes it, and nothing re-inserts the entry on a
    /// node that reaches neither the write-back nor a ladder rung. Keeping them
    /// is also what makes this half agree with the disk half, which retains every
    /// agreed record and rehydrates all of them at boot.
    ///
    /// Worst case that costs: one entry per committee change this process ever
    /// saw, at 304 B of `(epoch, key, tag)` plus its map node. A pathological
    /// chain that re-minted EVERY epoch at a day-long epoch holds ~365 of them a
    /// year — ~0.1 MB — against the ~101 B/record the journal already accepts for
    /// the same set on disk.
    pub fn retain_from(&self, oldest: u64) {
        if let Ok(mut m) = self.map.write() {
            m.retain(|e, (_, src)| *e >= oldest || *src == KeySource::Agreed);
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
    /// handle consumes any permit stored by a [`set_pk`](Self::set_pk) that fired
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

/// How an agreed `PK_epoch` is obtained for the epoch that MINTED it. Two
/// implementations, both closures so this module stays free of the store and
/// resolver types: a local read of the artifact store, and one bounded pull from
/// a peer.
pub(crate) type AgreedKeyAt =
    Arc<dyn Fn(u64) -> BoxFuture<'static, Option<GroupPublic>> + Send + Sync>;

/// A ladder rung over the agreement plane's artifact, ready to run.
///
/// The artifact is keyed by the epoch whose committee agreed it, which is the
/// epoch that MINTED the key — not necessarily the epoch being asked about. A
/// stable epoch runs no agreement at all and carries its predecessor's key
/// forward, so the rung first asks the chain which epoch minted
/// ([`chain_key_epoch`], the immutable `dkgQual` record) and then asks for that
/// epoch's artifact. Bundled with its reader because a caller holding one without
/// the other would have to re-derive the carry-forward walk — the duplicate this
/// module exists to stop.
#[derive(Clone)]
pub struct AgreedKeys {
    at: AgreedKeyAt,
    dkg_qual: DkgQualFor,
    /// Memo for the `epoch → minting epoch` walk. Per-instance and shared by
    /// clone, so every rung built over the same plane shares one. See
    /// [`chain_key_epoch_memoised`] for why a successful answer is eternal and a
    /// `None` must never land here.
    ///
    /// [`chain_key_epoch_memoised`]: super::carry::chain_key_epoch_memoised
    carry_memo: Arc<Mutex<BTreeMap<u64, u64>>>,
}

impl AgreedKeys {
    pub fn new(at: AgreedKeyAt, dkg_qual: DkgQualFor) -> Self {
        Self {
            at,
            dkg_qual,
            carry_memo: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// `(minting epoch, key)` for the key in force at `epoch`.
    ///
    /// `None` has three causes and every one of them is "not now", never "no
    /// key": the `dkgQual` read is undecided (a node with no finalized marker
    /// yet), `epoch` predates the beacon entirely, or no artifact for the
    /// minting epoch is reachable through this rung. The caller's answer to all
    /// three is the same — stay unpinned, i.e. vote-only admission — so they are
    /// deliberately not distinguished here.
    pub async fn key_for(&self, epoch: u64) -> Option<(u64, GroupPublic)> {
        let minted_at = chain_key_epoch_memoised(epoch, &self.dkg_qual, &self.carry_memo)??;
        let pk = (self.at)(minted_at).await?;
        debug!(
            epoch,
            minted_at,
            group_public = %pk_prefix(&pk),
            "resolved PK_epoch from the agreement artifact"
        );
        Some((minted_at, pk))
    }
}

/// Which rungs a caller may spend on a [`BeaconKeys::get_pk`]. The store rung is
/// unconditional; everything else is opt-in, because the rungs differ by ORDERS
/// of latency and the caller is the only one who knows its budget.
///
/// Absent rung ⇒ that source is simply not consulted. Never pass `pull` from a
/// caller on the vote path: its budget is seconds against that path's one.
#[derive(Default, Clone, Copy)]
pub struct KeySources<'a> {
    /// The agreement artifacts this node already holds. Memory, then disk.
    pub held: Option<&'a AgreedKeys>,
    /// One bounded pull of the minting epoch's artifact from a peer. Network.
    pub pull: Option<&'a AgreedKeys>,
    /// Weakest provenance the STORE rung may answer with; `None` ⇒ any recorded
    /// entry, which is what a caller whose answer is revisable wants.
    ///
    /// This is the ONE rung that is not a source but a cache OVER the sources,
    /// so it is the one rung that can hand back a tier the caller would never
    /// have accepted from the source itself. A floor here does not remove the
    /// rung — the stronger tiers still short-circuit the walk and the fetch; it
    /// only stops a weak entry from pre-empting a better answer that the later
    /// rungs would have produced.
    pub store_floor: Option<KeySource>,
}

/// The ONE `PK_epoch` ladder, and its order is load-bearing:
///
/// 1. **the shared store** — W1 (own key, published before the engine spawns)
///    and the agreement plane's write-back both land here;
/// 2. **the artifacts this node already holds** ([`AgreedKeys`] over the local
///    store);
/// 3. **one bounded pull** of the minting epoch's artifact from a peer.
///
/// Store-first because both later rungs cost a `dkgQual` chain read even when
/// they hit, and a caller that already has the answer must not pay for it. The
/// store is also the ONLY rung that mixes provenance tiers — it memoises the
/// later rungs' `Agreed`/`Carried` answers alongside W1/W3's locally
/// reconstructed ones — so a caller that cannot take a local reconstruction
/// raises [`KeySources::store_floor`] rather than dropping the rung.
///
/// **There is no rung for this node's own DKG material, and its absence is
/// load-bearing.** Everything this ladder resolves is a VERIFY-side value — the
/// seed pin a wire-received certificate is checked against; no signing decision
/// reads it. On that side the two failure directions are not symmetric. A pin is
/// write-once and terminal when wrong: `EpochSchemeProvider` refuses a later
/// registration that drops one, and every seedless certificate of that epoch is
/// then rejected for the life of the process. A MISSING pin only degrades the
/// epoch to vote-only admission, which still verifies the attributable multisig
/// quorum, committee membership and subject binding — it loses detection of a
/// tampered seed slot riding a valid quorum, and nothing else. A locally
/// reconstructed key can diverge from the network's (soak 2026-07-14), so a rung
/// handing one to the terminal side trades a recoverable failure for an undoable
/// one. Local material is still reachable where it is safe to take it: W1/W3
/// publish it into rung 1, and [`KeySources::store_floor`] lets each caller that
/// cannot take that tier exclude it — one mechanism the caller controls, rather
/// than a rung it could only take or drop whole.
///
/// `held`/`pull` are `None` where there is no artifact store to read or no route
/// to fetch over. **A `--cert-follow` follower is no longer such a caller**
/// (FLU-1167): [`crate::beacon::follower::for_follower`] gives it a RAM-only
/// artifact store of its own and a delivery route over its cert upstream — the
/// one peer relationship it has — so rung 3 exists there, it is just not the
/// plane's `BEACON_RESOLVER_CHANNEL` pull. It is the same [`AgreedKeys`] shape
/// over a different transport, checked by the same
/// [`crate::beacon::artifact::verify_artifact_for_epoch`] against
/// `committee[minted_at]` before anything is kept.
///
/// **Which rungs each of its two callers may spend is the whole design there,
/// and it is the rule stated above, not an exception to it.** The follower's
/// `ensure_key` runs per certificate and passes `pull: None` at BOTH efforts, so
/// the vote path stays network-free by construction; its background fetch task
/// passes `held` AND `pull`, off that path, on the same
/// [`crate::beacon::artifact::PULL_MIN_INTERVAL`] per-epoch budget the plane's
/// pull owes its peers. Rung 1 is what joins the two: a fetch that resolves
/// writes `Agreed` under the MINTING epoch and memoises `Carried` under the
/// asked-for one, exactly as it does on a validator, which is what makes the
/// next per-certificate ask a map hit. A follower's store therefore has ONE
/// writer — this ladder's own memoisation — where a validator's also has W1/W3
/// and the agreement write-back; it is no longer empty for the life of the
/// process, and its certificates take vote-only admission until the epoch's
/// artifact arrives rather than always. The quorum is fully verified either way;
/// the seed check is what it does without in the meantime.
///
/// The provenance floor does NOT differ by node class, and deliberately: both
/// follower callers pass the same `store_floor: Some(KeySource::Carried)` the
/// plane passes. A follower has no local reconstruction to floor out today —
/// nothing on that path writes [`KeySource::LocalDkg`] — but a floor that varied
/// by node class is exactly how one class quietly starts pinning a weaker tier
/// than the rule above admits.
impl BeaconKeys {
    /// The key in force at `epoch`, spending only the rungs `sources` permits.
    ///
    /// The ONE way to ask that question. [`Self::cached_only`] and
    /// [`Self::attested`] remain, and neither is a cheaper tier of this: the
    /// first answers "is anything recorded yet", the second "what did a quorum
    /// attest for this epoch". Only this one resolves.
    pub async fn get_pk(&self, epoch: u64, sources: KeySources<'_>) -> Option<GroupPublic> {
        let cached = match sources.store_floor {
            Some(min) => self.cached_at_least(epoch, min),
            None => self.cached_only(epoch),
        };
        if let Some(pk) = cached {
            return Some(pk);
        }
        for rung in [sources.held, sources.pull].into_iter().flatten() {
            if let Some((minted_at, pk)) = rung.key_for(epoch).await {
                // Filed under the epoch that MINTED it — the epoch whose
                // committee quorum actually signed the artifact. Filing it under
                // `epoch` would make `attested(epoch)` report a carried key as
                // directly attested for an epoch nobody attested it for.
                self.set_pk(minted_at, pk, KeySource::Agreed);
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
    /// [`KeySource::Carried`] and never `Agreed`: nobody attested this key FOR
    /// this epoch, and [`Self::attested`] is the promote value-gate's input.
    fn memoise_carry(&self, epoch: u64, minted_at: u64, pk: GroupPublic) {
        if epoch != minted_at {
            self.set_pk(epoch, pk, KeySource::Carried);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1};
    use fluentbase_bls::PeerPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// Distinct group keys, deterministically.
    fn pk(seed: u64) -> GroupPublic {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, _) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        *crate::beacon::outcome::group_public_key(&outcome)
    }

    /// An artifact rung over a canned store: records every epoch it was asked for,
    /// so "which epoch did it choose" and "did it ask at all" are both observable.
    fn rung(
        held: BTreeMap<u64, GroupPublic>,
        bits: &[u64],
        asked: Arc<std::sync::Mutex<Vec<u64>>>,
    ) -> AgreedKeys {
        let set: std::collections::BTreeSet<u64> = bits.iter().copied().collect();
        let dkg_qual: DkgQualFor = Arc::new(move |e| Some(set.contains(&e)));
        AgreedKeys::new(
            Arc::new(move |epoch: u64| {
                asked.lock().unwrap().push(epoch);
                let hit = held.get(&epoch).copied();
                Box::pin(async move { hit }) as BoxFuture<'static, Option<GroupPublic>>
            }),
            dkg_qual,
        )
    }

    /// The memo, and what it is for: the SECOND ask for a carried epoch must not
    /// pay for the chain read plus the store lookup again.
    ///
    /// Reds if `memoise_carry` stops writing.
    #[tokio::test]
    async fn a_carried_epoch_is_resolved_once_and_then_hit_in_the_map() {
        let bootstrap = crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
        let expected = pk(1);
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let held = rung(BTreeMap::from([(bootstrap, expected)]), &[], asked.clone());
        let store = BeaconKeys::new();
        let sources = KeySources {
            held: Some(&held),
            ..Default::default()
        };

        assert_eq!(store.get_pk(40, sources).await, Some(expected));
        assert_eq!(
            asked.lock().unwrap().len(),
            1,
            "the first ask really resolved"
        );

        assert_eq!(store.get_pk(40, sources).await, Some(expected));
        assert_eq!(
            asked.lock().unwrap().len(),
            1,
            "the second ask must not touch the artifact store at all"
        );
    }

    /// A committee stable since genesis is the devnet default, and it is exactly
    /// where the naive target is wrong: epoch 40 ran no agreement, so no artifact
    /// is keyed under it. The rung must ask for the BOOTSTRAP mint's epoch, and
    /// the key must come back filed under that epoch — not under the epoch being
    /// repaired, or [`BeaconKeys::attested`] would claim an attestation nobody
    /// made for it.
    ///
    /// Reds if the rung asks for `epoch` itself, and reds if the recording moves
    /// to the queried epoch.
    #[tokio::test]
    async fn a_stable_committee_asks_for_the_minting_epoch_and_files_it_there() {
        let bootstrap = crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
        let expected = pk(2);
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let held = rung(BTreeMap::from([(bootstrap, expected)]), &[], asked.clone());
        let store = BeaconKeys::new();

        let got = store
            .get_pk(
                12,
                KeySources {
                    held: Some(&held),
                    ..Default::default()
                },
            )
            .await;

        assert_eq!(got, Some(expected));
        assert_eq!(
            *asked.lock().unwrap(),
            vec![bootstrap],
            "the bootstrap mint's epoch, not epoch 12"
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

    /// A `dkgQual` bit SET at epoch 9 names 9 as the mint, so that is the epoch
    /// the rung must ask for — not the bootstrap and not the queried epoch.
    #[tokio::test]
    async fn a_set_qual_bit_names_the_minting_epoch() {
        let expected = pk(3);
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let held = rung(BTreeMap::from([(9, expected)]), &[9], asked.clone());
        let store = BeaconKeys::new();
        let got = store
            .get_pk(
                12,
                KeySources {
                    held: Some(&held),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(got, Some(expected));
        assert_eq!(*asked.lock().unwrap(), vec![9]);
    }

    /// An undecided `dkgQual` read must produce NO lookup at all. A node with no
    /// finalized marker yet reads exactly that, and asking for a guessed epoch
    /// would spend the network rung's one bounded pull on the wrong artifact.
    ///
    /// Reds if the undecided arm falls through to an epoch instead of returning.
    #[tokio::test]
    async fn an_undecided_chain_read_issues_no_lookup() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let unreadable: DkgQualFor = Arc::new(|_| None);
        let held = AgreedKeys::new(
            {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| {
                    asked.lock().unwrap().push(epoch);
                    Box::pin(async { None }) as BoxFuture<'static, Option<GroupPublic>>
                })
            },
            unreadable,
        );

        assert_eq!(held.key_for(12).await, None);
        assert!(
            asked.lock().unwrap().is_empty(),
            "an undecided read must not spend the lookup"
        );
    }

    /// The store rung answers before either artifact rung, so a caller that
    /// already has the key never pays the `dkgQual` chain read.
    #[tokio::test]
    async fn the_ladder_prefers_the_store_over_the_artifact_rungs() {
        let store = BeaconKeys::new();
        store.set_pk(40, pk(9), KeySource::LocalDkg);
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let held = rung(BTreeMap::new(), &[], asked.clone());
        assert_eq!(
            store
                .get_pk(
                    40,
                    KeySources {
                        held: Some(&held),
                        ..Default::default()
                    }
                )
                .await,
            Some(pk(9))
        );
        assert!(asked.lock().unwrap().is_empty());
    }

    /// The order below the store: held artifacts, then one network pull. An
    /// empty local store must not stop the pull, and the pulled value is filed
    /// at the minting epoch under `Agreed`.
    #[tokio::test]
    async fn the_ladder_falls_through_held_then_pull() {
        let bootstrap = crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
        let held_asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let pull_asked = Arc::new(std::sync::Mutex::new(Vec::new()));

        let expected = pk(4);
        let store = BeaconKeys::new();
        assert_eq!(
            store
                .get_pk(
                    40,
                    KeySources {
                        held: Some(&rung(BTreeMap::new(), &[], held_asked.clone())),
                        pull: Some(&rung(
                            BTreeMap::from([(bootstrap, expected)]),
                            &[],
                            pull_asked.clone()
                        )),
                        store_floor: None,
                    }
                )
                .await,
            Some(expected)
        );
        assert_eq!(
            *held_asked.lock().unwrap(),
            vec![bootstrap],
            "held asked first"
        );
        assert_eq!(
            *pull_asked.lock().unwrap(),
            vec![bootstrap],
            "then the pull"
        );
        assert_eq!(store.attested(bootstrap), Some(expected));
    }

    /// The soak-2026-07-14 inversion, fixed: a stale LOCAL write landed first,
    /// then the network's attested key arrived and was DROPPED by
    /// first-write-wins — the poisoned entry then failed the next epoch's
    /// parent-seed witness. Attested-source-wins: the quorum-agreed write must
    /// DISPLACE the differing local one.
    #[test]
    fn an_agreed_key_displaces_a_differing_local_write() {
        let (local, network) = (pk(1), pk(2));
        assert_ne!(local, network);
        let map = BeaconKeys::new();
        map.set_pk(77, local, KeySource::LocalDkg); // W1 (stale)
        map.set_pk(77, network, KeySource::Agreed);
        assert_eq!(
            map.cached_only(77),
            Some(network),
            "a quorum-agreed key must beat local reconstruction regardless of timing"
        );
        assert_eq!(map.attested(77), Some(network));
    }

    /// The carried tier sits between the other two, and both directions matter.
    /// A carry is derived from two CHAIN facts, so it must beat a local
    /// reconstruction that soak 2026-07-14 proved can diverge; and nobody
    /// attested it FOR this epoch, so an agreed write must still beat it.
    ///
    /// Reds if `Carried` is declared anywhere other than between `LocalDkg` and
    /// `Agreed` — the conflict policy reads strength through `Ord`, so the
    /// variant's POSITION is the policy.
    #[test]
    fn carried_beats_local_and_loses_to_agreed() {
        let (local, carried, network) = (pk(1), pk(2), pk(3));
        let map = BeaconKeys::new();

        map.set_pk(77, local, KeySource::LocalDkg);
        map.set_pk(77, carried, KeySource::Carried);
        assert_eq!(
            map.cached_only(77),
            Some(carried),
            "a carry is two chain facts; a local reconstruction is neither"
        );

        map.set_pk(77, network, KeySource::Agreed);
        assert_eq!(
            map.cached_only(77),
            Some(network),
            "and the epoch's own attestation beats a key carried into it"
        );

        // The reverse direction, on a fresh entry: an agreed key already held is
        // not displaced by a carry.
        let map = BeaconKeys::new();
        map.set_pk(77, network, KeySource::Agreed);
        map.set_pk(77, carried, KeySource::Carried);
        assert_eq!(map.cached_only(77), Some(network));
    }

    /// `attested` is the promote value-gate's input, and the gate compares a
    /// LOCAL resolve against what a quorum attested. A carried key is not that:
    /// it was attested for the minting epoch, not for this one. If it became
    /// visible here the gate would compare a derived value against itself and
    /// pass anything.
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

    /// The inverse ordering: once an agreed key holds the entry, no later local
    /// resolve may displace it (last-write-wins would re-open the forge arm the
    /// insert-only store exists to close).
    #[test]
    fn local_write_never_displaces_an_attested_entry() {
        let (local, network) = (pk(1), pk(2));
        let map = BeaconKeys::new();
        map.set_pk(77, network, KeySource::Agreed);
        map.set_pk(77, local, KeySource::LocalDkg);
        assert_eq!(map.cached_only(77), Some(network));
        assert_eq!(map.attested(77), Some(network));
    }

    /// Same value, stronger provenance: an agreed confirm UPGRADES a local entry
    /// to attested (the promote value-gate's input); within a tier the first
    /// write wins and the store stays insert-only.
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
        map.set_pk(9, key, KeySource::Agreed);
        assert_eq!(map.attested(9), Some(key));
        // And it never downgrades back.
        map.set_pk(9, key, KeySource::LocalDkg);
        assert_eq!(map.attested(9), Some(key));
    }

    /// The promote value-gate compares ONLY against attested entries: a differing
    /// local entry (our own earlier write — possibly the same stale source) must
    /// not masquerade as a quorum observation.
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
}
