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
/// Public for a second consumer beyond [`select_carry_scheme`]: the boundary
/// FETCH has to name one height and cannot walk. `Epocher::first(epoch)` is the
/// right height only where `epoch` itself minted — on a committee that has been
/// stable for a while, that block carries no outcome and the fetch returns
/// something useless. This is the only way to name the height that does carry it.
///
/// Note the two `Option` layers, and do not flatten them at a call site: the
/// outer is "the chain could not be read, retry later" and the inner is "this
/// epoch predates the beacon entirely". Conflating them turns a transient into a
/// permanent verdict.
pub fn chain_key_epoch(epoch: u64, dkg_qual: &DkgQualFor) -> Option<Option<u64>> {
    if epoch < DETERMINISTIC_BOOTSTRAP_EPOCH {
        return Some(None); // seedless pre-beacon epochs — nothing to serve
    }
    for e in (DETERMINISTIC_BOOTSTRAP_EPOCH + 1..=epoch).rev() {
        match dkg_qual(e) {
            Some(true) => return Some(Some(e)),
            Some(false) => continue,
            None => return None,
        }
    }
    Some(Some(DETERMINISTIC_BOOTSTRAP_EPOCH))
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
pub type DkgQualProbe = Arc<dyn Fn(u64, B256) -> Option<(bool, bool)> + Send + Sync>;

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
