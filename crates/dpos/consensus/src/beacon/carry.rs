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
//! committee reads (whose pruning forced the former async block rung). A newer
//! local mint the chain DECLINED (its epoch's bit is clear — soak v47) is simply
//! UNUSED; a missed re-mint during downtime (departure-then-backfill, soak
//! 2026-07-14) is a set bit in the span the node holds no mint for ⇒ refuse.
//! The network-attested-key divergence tripwire in the resolvers stays as the
//! defense-in-depth backstop.

use super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
use std::sync::Arc;

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
fn chain_key_epoch(epoch: u64, dkg_qual: &DkgQualFor) -> Option<Option<u64>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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
}
