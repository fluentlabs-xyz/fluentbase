//! On-chain `dkgQual`-bit arbitration of the beacon-key CARRY-FORWARD (§8.11.1).
//!
//! `dkgQual[e]` is set DETERMINISTICALLY by the contract at `commitEpochCommittee`
//! (`dkgQual[e] = committee[e] != committee[e−1]`) and never mutated after, so the
//! bit history is an immutable on-chain record of exactly which epochs the network
//! re-minted at: set ⇔ the committee changed at `e` and its DKG re-minted the key
//! (a mint at `e`); clear ⇔ the committee was carried/stable (no mint at `e`). The
//! deterministic bootstrap epoch mints UNCONDITIONALLY. The chain's key epoch for
//! `E` is therefore a pure chain fact:
//!
//!   `chain_key_epoch(E) = last e in (BOOTSTRAP, E] with dkgQual[e], else BOOTSTRAP`
//!
//! and a node either holds the mint stored at exactly that epoch or holds no
//! usable material for `E` (verify-only until the recompute-heal lands).
//!
//! Soundness: no bit in `(m, E]` ⇒ every commit in the span carried ⇒
//! `committee[E] == committee[m]`, and the AM5 agreed-dealing-set makes every
//! honest mint at `m` byte-identical (players == the committed candidate), so
//! serving the stored mint at `chain_key_epoch(E)` IS serving the chain's
//! current key — no player-set comparison, no span proof, and no on-chain
//! committee reads (whose pruning, since retired, forced the former async block
//! rung). A newer
//! local mint the chain DECLINED (its epoch's bit is clear — soak v47) is simply
//! UNUSED; a missed re-mint during downtime (departure-then-backfill, soak
//! 2026-07-14) is a set bit in the span the node holds no mint for ⇒ refuse.
//! The network-attested-key divergence tripwire in the resolvers stays as the
//! defense-in-depth backstop.

use super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
use alloy_primitives::B256;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Read the FROZEN on-chain `dkgQual[e]` bit. `None` = could not read
/// (transient) — the caller must treat the resolve as undecided, never as
/// "no re-mint". Implementations read at a finalized hash and may cache any
/// bit whose epoch's committee is already committed (frozen forever).
pub type DkgQualFor = Arc<dyn Fn(u64) -> Option<bool> + Send + Sync>;

/// Verdict of the carry-forward arbitration for a target epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarryVerdict {
    /// The chain's key epoch for the target is `minted_at` and the node holds
    /// that mint. Serve it.
    Serve { minted_at: u64 },
    /// The node holds no mint at the chain's key epoch (never attended it, or
    /// its own newer mint was declined on-chain) — genuine "no usable material",
    /// a structural fact: demote to verify-only, heal via recompute.
    NoUsableMint,
    /// A `dkgQual` read failed (transient) — could not decide; retried on the
    /// next resolve edge.
    ReadFailed,
}

/// The chain's key epoch for `epoch`: the last `e` in
/// `(DETERMINISTIC_BOOTSTRAP_EPOCH, epoch]` with `dkgQual[e]` set, else the
/// bootstrap epoch. `None` when a bit read fails (undecided) or when `epoch`
/// predates the bootstrap mint (no beacon exists at all).
///
/// Visible to a second consumer beyond [`select_carry_scheme`]: the agreement
/// rung [`crate::beacon::keys::AgreedKeys::key_for`], which has to name ONE
/// epoch's artifact and cannot walk. The artifact is keyed by the epoch that
/// MINTED the key, and `epoch` itself is the minting epoch only where it
/// re-minted — a committee stable for a while ran no agreement at all, so asking
/// for `epoch`'s own artifact returns nothing. This is the only way to name the
/// epoch that does hold it.
///
/// Note the two `Option` layers, and do not flatten them at a call site: the
/// outer is "the chain could not be read, retry later" and the inner is "this
/// epoch predates the beacon entirely". Conflating them turns a transient into a
/// permanent verdict.
pub(crate) fn chain_key_epoch(epoch: u64, dkg_qual: &DkgQualFor) -> Option<Option<u64>> {
    chain_key_epoch_memoised(epoch, dkg_qual, &Mutex::new(BTreeMap::new()))
}

/// [`chain_key_epoch`] with the answer memoised, and the memo is what makes the
/// provenance floor affordable on a per-certificate path.
///
/// **Why a SUCCESSFUL answer is eternal, so caching it adds no new trust.** Every
/// bit this scan reads comes back through [`frozen_dkg_qual`], which answers
/// `Some` only once the bit is DECIDED — set, or its epoch's committee committed —
/// and `None` otherwise. A `None` aborts the scan. So a `Some` answer is a
/// function of frozen facts alone and cannot change later. This holds for ANY
/// epoch that yields an answer, not merely for old ones.
///
/// **`None` is NEVER memoised, and that is not tidiness.** A catching-up node
/// reads at a finalized hash far behind the chain: for an epoch whose committee is
/// not yet committed there, the bit reads as a default `false` with an empty
/// committee, and `frozen_dkg_qual` correctly says "undecided". Recording that as
/// an answer would pin the bootstrap key onto that epoch permanently — and a pin
/// is write-once, so every seedless certificate of that epoch would be rejected
/// for the life of the process. A recoverable retry turned into an unrecoverable
/// refusal.
///
/// **Incremental, not a plain cache.** `chain_key_epoch(E) = max{ e <= E :
/// dkgQual[e] }` is monotone in `E`, so a miss scans down only to the first
/// memoised epoch and inherits its answer. Total work over a process is O(epochs),
/// per call usually zero or one step — where the unmemoised scan costs `E -
/// BOOTSTRAP` iterations EVERY time on a stable committee, because the answer sits
/// at the bootstrap epoch and nothing above it is ever set. That is the shape that
/// halved the devnet block rate when the floor put this walk behind every
/// certificate.
///
/// Capping the walk instead is not an option and was already tried: on a stable
/// committee the correct answer is arbitrarily deep, so any cap yields either a
/// permanent retry or a false bootstrap answer — the same terminal pin. The
/// retired `CARRY_WALK_CAP` was removed WITHOUT replacement for exactly this.
pub(crate) fn chain_key_epoch_memoised(
    epoch: u64,
    dkg_qual: &DkgQualFor,
    memo: &Mutex<BTreeMap<u64, u64>>,
) -> Option<Option<u64>> {
    if epoch < DETERMINISTIC_BOOTSTRAP_EPOCH {
        // Seedless pre-beacon epochs — nothing to serve.
        //
        // A SECOND CONSUMER DEPENDS ON THIS LINE, and not for its own answer:
        // `PlaneRandomness::signer_scheme`'s `Signs` arm builds its oracle with
        // `oracle_at`, bypassing the `mandatory_at` door in `oracle_for`, and is
        // safe only because this `Some(None)` makes the share resolver answer
        // `Absent` here — so `material` is `None` and no oracle is attached. An
        // oracle on a pre-beacon epoch refuses every LEGAL seedless certificate
        // of it, so if this refusal ever moves, that arm has to gain the gate.
        return Some(None);
    }
    if let Some(hit) = memo.lock().ok().and_then(|m| m.get(&epoch).copied()) {
        return Some(Some(hit));
    }
    let mut answer = None;
    for e in (DETERMINISTIC_BOOTSTRAP_EPOCH + 1..=epoch).rev() {
        // A memoised LOWER epoch answers this one too, by monotonicity: nothing
        // between it and `epoch` had its bit set, or the scan would have stopped.
        if let Some(hit) = memo.lock().ok().and_then(|m| m.get(&e).copied()) {
            answer = Some(hit);
            break;
        }
        match dkg_qual(e) {
            Some(true) => {
                answer = Some(e);
                break;
            }
            Some(false) => continue,
            // Undecided: abort WITHOUT recording anything. See the doc above.
            None => return None,
        }
    }
    let minted_at = answer.unwrap_or(DETERMINISTIC_BOOTSTRAP_EPOCH);
    if let Ok(mut m) = memo.lock() {
        m.insert(epoch, minted_at);
    }
    Some(Some(minted_at))
}

/// Arbitrate which stored mint (if any) this node serves for `epoch`.
/// `has_mint(e)` answers whether the local `CeremonyStore` holds the ceremony
/// minted at `e`.
pub fn select_carry_scheme(
    epoch: u64,
    has_mint: impl Fn(u64) -> bool,
    dkg_qual: &DkgQualFor,
) -> CarryVerdict {
    match chain_key_epoch(epoch, dkg_qual) {
        None => CarryVerdict::ReadFailed,
        Some(None) => CarryVerdict::NoUsableMint,
        Some(Some(minted_at)) => {
            if has_mint(minted_at) {
                CarryVerdict::Serve { minted_at }
            } else {
                CarryVerdict::NoUsableMint
            }
        }
    }
}

/// One chain probe for [`frozen_dkg_qual`]: `(dkgQual[epoch], is the epoch's
/// committee committed)` read at `at`, or `None` when the bit itself could not be
/// read. The committee leg answers only whether the epoch exists on-chain yet —
/// a committee read fault reports `false` (not yet committed), which is the same
/// undecided outcome by a different route.
pub(crate) type DkgQualProbe = Arc<dyn Fn(u64, B256) -> Option<(bool, bool)> + Send + Sync>;

/// Build the FROZEN [`DkgQualFor`] reader over a chain-state probe.
///
/// The bit is immutable once its epoch's committee is committed, so a decided
/// answer is memoised forever and a long stable span costs ONE state read per
/// epoch across the process's life rather than one per resolve.
///
/// **`frozen` decides the ANSWER, not just the caching.** An epoch whose
/// committee is not committed yet reads its bit as the contract map's default
/// `false` and its committee as empty; reporting `Some(false)` there fabricates
/// "no re-mint at E" for an epoch the chain does not have, and both consumers
/// then serve a CARRIED key under it. [`DkgQualFor`]'s contract says the honest
/// answer is `None` — undecided, retry — and both consumers already handle that
/// non-fatally (`KeyLookup::ReadFailed` = retry; `BeaconResolve::Absent` = the
/// share gate demotes and the recompute-heal promotes later).
///
/// That the non-empty-committee leg carries ONE meaning ("already committed")
/// became true only with FLU-1134. Under the retired committee pruning it also
/// meant "not yet pruned", so a deep epoch read back empty, a clear bit was
/// never frozen and never cached, and the `chain_key_epoch` scan re-read every
/// epoch from E down to the last change on every single call — which is exactly
/// the long stable span this memo exists for.
///
/// The memo is deliberately NOT bounded by a trailing epoch window, unlike the
/// crate's other per-epoch maps. [`chain_key_epoch`] scans DOWNWARD from the
/// queried epoch to the last set bit, and on the stable committee this memo
/// exists for that bit is the bootstrap epoch — so while no bit is set in the
/// span, every entry between the bootstrap and the tip is on the scan path and is
/// re-read the moment it is evicted. (One set bit at `m` makes everything below
/// `m` unreachable and safely evictable — the unbounded case is the all-clear
/// history, which is exactly the one this memo serves.) A window of `SCHEME_RETENTION_EPOCHS` would therefore restore the
/// pre-FLU-1134 behaviour described above (one chain read per epoch of the span,
/// per call) on a path the vote gate takes per block, to reclaim a map that grows
/// by one `(u64, bool)` per epoch. How much that is depends on `epochBlockInterval`,
/// a contract-read chain parameter and not a code constant: hundreds of bytes a year
/// at day-long epochs, two to three orders of magnitude more at the devnet's 32.
///
/// Shared by both launch paths deliberately: the validator builds it over its
/// beacon plane's reader and the follower over its own, and the two differ in
/// nothing but the reader instance. Duplicating the freeze rule is how one copy
/// drifts.
pub fn frozen_dkg_qual(
    at_finalized: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
    probe: DkgQualProbe,
) -> DkgQualFor {
    let cache: Arc<Mutex<BTreeMap<u64, bool>>> = Arc::new(Mutex::new(BTreeMap::new()));
    Arc::new(move |epoch: u64| {
        if let Some(v) = cache.lock().ok().and_then(|c| c.get(&epoch).copied()) {
            return Some(v);
        }
        let at = at_finalized()?;
        let (bit, committed) = probe(epoch, at)?;
        if !(bit || committed) {
            return None;
        }
        if let Ok(mut c) = cache.lock() {
            c.insert(epoch, bit);
        }
        Some(bit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The memo must never record an UNDECIDED epoch, and the cost of getting this
    /// wrong is not a stale read — it is a permanent refusal.
    ///
    /// A catching-up node reads at a finalized hash far behind the chain. For an
    /// epoch whose committee is not committed there, the bit reads as a default
    /// `false` over an empty committee, and `frozen_dkg_qual` correctly answers
    /// "undecided". Recording that would pin the bootstrap key onto that epoch
    /// forever — and a pin is write-once, so every seedless certificate of the
    /// epoch is rejected for the life of the process.
    #[test]
    fn an_undecided_epoch_is_never_memoised_and_stays_retryable() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let decided = Arc::new(AtomicBool::new(false));
        let d = decided.clone();
        // Epoch 5's bit is undecided until `decided` flips, then it is set.
        let qual: DkgQualFor = Arc::new(move |e: u64| {
            if e == 5 {
                return d.load(Ordering::SeqCst).then_some(true);
            }
            Some(false)
        });
        let memo = Mutex::new(BTreeMap::new());

        assert_eq!(
            chain_key_epoch_memoised(9, &qual, &memo),
            None,
            "an undecided bit in the span makes the whole answer undecided"
        );
        assert!(
            memo.lock().unwrap().is_empty(),
            "nothing may be recorded from a scan that hit an undecided bit"
        );

        decided.store(true, Ordering::SeqCst);
        assert_eq!(
            chain_key_epoch_memoised(9, &qual, &memo),
            Some(Some(5)),
            "once decided, the same call resolves — proving the earlier None was not cached"
        );
    }

    /// The memo is INCREMENTAL: a later epoch inherits a memoised earlier answer
    /// instead of re-walking to it. That is what makes the per-certificate cost a
    /// step rather than the whole epoch range.
    #[test]
    fn a_memoised_lower_epoch_answers_a_higher_one_without_re_reading_the_span() {
        let reads = Arc::new(AtomicUsize::new(0));
        let r = reads.clone();
        let qual: DkgQualFor = Arc::new(move |e: u64| {
            r.fetch_add(1, Ordering::SeqCst);
            Some(e == 4)
        });
        let memo = Mutex::new(BTreeMap::new());

        assert_eq!(chain_key_epoch_memoised(20, &qual, &memo), Some(Some(4)));
        let first = reads.load(Ordering::SeqCst);
        assert!(
            first > 1,
            "premise: the first call really did walk the span ({first} reads)"
        );

        // A HIGHER epoch: the walk should meet the memoised 20 after one step and
        // stop, not descend to 4 again.
        assert_eq!(chain_key_epoch_memoised(21, &qual, &memo), Some(Some(4)));
        assert_eq!(
            reads.load(Ordering::SeqCst) - first,
            1,
            "one step down to the memoised epoch, not a fresh walk"
        );

        // And the exact epoch is a pure hit.
        assert_eq!(chain_key_epoch_memoised(20, &qual, &memo), Some(Some(4)));
        assert_eq!(
            reads.load(Ordering::SeqCst) - first,
            1,
            "a memoised epoch costs no chain read at all"
        );
    }

    fn qual(bits: &[u64]) -> DkgQualFor {
        let set: std::collections::BTreeSet<u64> = bits.iter().copied().collect();
        Arc::new(move |e| Some(set.contains(&e)))
    }

    fn select(epoch: u64, mints: &[u64], dkg_qual: &DkgQualFor) -> CarryVerdict {
        let store: BTreeMap<u64, ()> = mints.iter().map(|m| (*m, ())).collect();
        select_carry_scheme(epoch, |e| store.contains_key(&e), dkg_qual)
    }

    /// The legitimate common case: a stable committee (no bits) carries the
    /// bootstrap mint forward across many epochs (the R1 rolling-restart
    /// guarantee).
    #[test]
    fn stable_committee_serves_carried_bootstrap_mint() {
        assert_eq!(
            select(12, &[2], &qual(&[])),
            CarryVerdict::Serve { minted_at: 2 }
        );
    }

    /// Soak v47: the candidate DKG under-qualified on-chain (bit clear), the
    /// contract re-committed the incumbent. The node's newer local mint at 5 is
    /// UNUSED — the carried mint at 3 (bit set) is served. The pre-fix code
    /// refused here and froze the whole committee.
    #[test]
    fn declined_candidate_mint_is_skipped_carried_mint_served() {
        assert_eq!(
            select(5, &[2, 3, 5], &qual(&[3])),
            CarryVerdict::Serve { minted_at: 3 }
        );
    }

    /// A candidate DKG that DID qualify on-chain: its bit is set, the fresh
    /// mint is served.
    #[test]
    fn qualified_candidate_mint_is_served_exact_epoch() {
        assert_eq!(
            select(5, &[2, 5], &qual(&[5])),
            CarryVerdict::Serve { minted_at: 5 }
        );
    }

    /// The A→B→A committee sandwich (departure-then-backfill, soak 2026-07-14):
    /// the network re-minted at 8 and 11 while this node held only the mint at
    /// 5 — the chain's key epoch is 11, which the node never attended ⇒ no
    /// usable material (the former span-proof machinery existed only for this).
    #[test]
    fn missed_remint_across_sandwich_is_unusable() {
        assert_eq!(
            select(12, &[2, 5], &qual(&[5, 8, 11])),
            CarryVerdict::NoUsableMint
        );
    }

    /// The chain's key epoch names a mint this node never attended (fresh
    /// joiner / observer) ⇒ no usable material.
    #[test]
    fn key_epoch_not_in_store_is_unusable() {
        assert_eq!(select(12, &[5], &qual(&[8])), CarryVerdict::NoUsableMint);
        // ... including the bootstrap fallback when the store is empty.
        assert_eq!(select(12, &[], &qual(&[])), CarryVerdict::NoUsableMint);
    }

    /// Seedless pre-beacon epochs (< bootstrap) resolve to nothing.
    #[test]
    fn pre_bootstrap_epoch_is_unusable() {
        assert_eq!(select(1, &[], &qual(&[])), CarryVerdict::NoUsableMint);
    }

    /// A failed bit read is UNDECIDED, never "no re-mint" — the resolve is
    /// retried, the node must not carry across an unreadable span.
    #[test]
    fn unreadable_bit_is_read_failed() {
        let holey: DkgQualFor = Arc::new(|e| (e != 7).then_some(false));
        let store: BTreeMap<u64, ()> = BTreeMap::from([(2, ())]);
        assert_eq!(
            select_carry_scheme(12, |e| store.contains_key(&e), &holey),
            CarryVerdict::ReadFailed
        );
    }

    /// The scan stops at the NEWEST set bit — an older mint in the store never
    /// shadows a newer chain re-mint.
    #[test]
    fn newest_bit_wins() {
        assert_eq!(
            select(12, &[2, 5, 9], &qual(&[5, 9])),
            CarryVerdict::Serve { minted_at: 9 }
        );
    }

    /// The whole point of the memo: `chain_key_epoch` walks the span from E down
    /// to the last change on EVERY resolve, so an un-memoised clear bit costs one
    /// state read per stable epoch per resolve. Reds if the cache stops recording
    /// a decided clear bit.
    #[test]
    fn a_decided_bit_is_read_from_the_chain_exactly_once() {
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = reads.clone();
        let dkg_qual = frozen_dkg_qual(
            Arc::new(|| Some(B256::repeat_byte(1))),
            Arc::new(move |_, _| {
                counted.fetch_add(1, Ordering::Relaxed);
                Some((false, true))
            }),
        );
        assert_eq!(dkg_qual(9), Some(false));
        assert_eq!(dkg_qual(9), Some(false));
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    /// An epoch whose committee is not committed yet reads a DEFAULT-`false` bit
    /// off the contract map. Reporting that as "no re-mint" fabricates chain
    /// history for an epoch the chain does not have, and the carry arbitration
    /// then serves a key under it. Reds if the `frozen` leg stops gating the
    /// answer (as opposed to only the caching).
    #[test]
    fn an_uncommitted_epoch_is_undecided_and_is_not_memoised() {
        let committed = Arc::new(AtomicUsize::new(0));
        let flips = committed.clone();
        let dkg_qual = frozen_dkg_qual(
            Arc::new(|| Some(B256::repeat_byte(1))),
            Arc::new(move |_, _| Some((false, flips.load(Ordering::Relaxed) > 0))),
        );
        assert_eq!(dkg_qual(9), None);
        committed.store(1, Ordering::Relaxed);
        assert_eq!(
            dkg_qual(9),
            Some(false),
            "the undecided answer must not have been cached"
        );
    }

    /// No finalized block yet (a follower before its first landing) is a read
    /// fault, not a clear bit. Reds if `at_finalized` is defaulted instead of
    /// short-circuiting.
    #[test]
    fn no_finalized_anchor_is_undecided() {
        let dkg_qual = frozen_dkg_qual(
            Arc::new(|| None),
            Arc::new(|_, _| panic!("must not probe without a finalized anchor")),
        );
        assert_eq!(dkg_qual(9), None);
    }
}
