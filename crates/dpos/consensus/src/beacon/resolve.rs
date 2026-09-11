//! Resolving `PK_epoch` from THIS node's own material.
//!
//! This used to be split across the consensus core: the 3-state lookup sat in
//! `application.rs` and the two closures that actually read the ceremony store
//! were built at the launch site in `dpos.rs`. It answers one question — "does
//! this node hold the epoch's group key" — out of DKG material, carry-forward
//! arbitration and an agreement store, none of which the core is allowed to
//! know exists.

use super::{
    carry::{select_carry_scheme, CarryVerdict, DkgQualFor},
    keys::BeaconKeys,
    surface::{BeaconResolve, BeaconResolver},
};
use crate::beacon::actor::CeremonyStore;
use fluentbase_bls::beacon::GroupPublic;
use std::sync::Arc;

/// Whether the mint this node stored at `minted_at` disagrees with the key a
/// `committee[minted_at]` quorum ATTESTED for that mint.
///
/// The on-chain `dkgQual` arbitration proves only that the chain minted AT
/// `minted_at`, NOT that this node's local outcome at that mint matches the
/// chain's. A member that finalized a divergent outcome (a torn/superset log set
/// → a different `Logs::select` → a different `PK_E`; see `recompute_scoped`)
/// holds a self-derived key nobody else signs with, and serving it drives a lone
/// `reject{bad_signature}` that splits honest voters (soak v39).
///
/// **Keyed on `minted_at`, never on the target epoch**, and that is the whole
/// reason this is one function instead of three inline compares.
/// [`BeaconKeys::attested`] answers only for the `Agreed` tier, and a CARRY epoch
/// runs no agreement of its own — so comparing at the target epoch is vacuously
/// `None` on exactly the epochs the carry serves, and the guard silently does
/// nothing. Comparing at the mint makes ONE observed outcome demote the mint and
/// every stable epoch that carries it. Absent attestation ⇒ no divergence (the
/// restarted-signer carry).
///
/// Pure: the callers own what a `true` costs (a metric, a log, their own verdict),
/// because the same divergence means "demote" on a reconcile path and only
/// "answer nothing" on the per-vote path.
pub(crate) fn mint_diverges_from_attested(
    group_keys: &BeaconKeys,
    minted_at: u64,
    local: &GroupPublic,
) -> bool {
    group_keys
        .attested(minted_at)
        .is_some_and(|net| net != *local)
}

/// Build the share-gate beacon resolver over the live-DKG `CeremonyStore`:
/// each vote carries the seed partial (round-keyed), so the seed is recovered
/// from the notarization/finalization certificate — no separate seed plane.
/// The key ROTATES per epoch: the store holds `(PK_E, share)` for every mint
/// this node attended; `resolve(E)` returns the most-recent such key at or
/// before E (carry-forward across stable epochs). There is NO genesis-baked
/// fallback key (epoch 1 is seedless; the first key is the deterministic
/// epoch-2 live DKG), so the resolver bottoms out at `Absent`.
///
/// A stored share is carried forward to E under ON-CHAIN `dkgQual`-BIT
/// ARBITRATION (`beacon::carry::select_carry_scheme`): the chain's key epoch
/// for E is the last set bit in `(BOOTSTRAP, E]` (else the bootstrap mint) and
/// the node serves its stored mint at exactly that epoch. A newer local mint
/// the chain declined (bit clear — soak v47) is UNUSED; a re-mint missed during
/// downtime (departure-then-backfill, soak 2026-07-14, v5@epoch77
/// reject{bad_signature}) is a set bit the store misses ⇒ refuse. A refusal
/// (`Absent`) → the `epoch_manager` share-gate demotes to verify-only and the
/// recompute-heal later promotes with the correct key.
pub(crate) fn beacon_share_resolver(
    store: CeremonyStore,
    dkg_qual: DkgQualFor,
    namespace: Vec<u8>,
    group_keys: BeaconKeys,
) -> BeaconResolver {
    Arc::new(move |epoch: u64| {
        let Ok(m) = store.read() else {
            return BeaconResolve::Absent;
        };
        // No local material at or below E — benign `Absent` (an observer /
        // pre-ceremony node), NOT a divergence: emit no refusal metric.
        if m.range(..=epoch).next_back().is_none() {
            return BeaconResolve::Absent;
        }
        match select_carry_scheme(epoch, |e| m.contains_key(&e), &dkg_qual) {
            CarryVerdict::Serve { minted_at } => {
                let (out, share) = m.get(&minted_at).expect("select returned a stored mint");
                // SHARE-GATE carry-divergence guard: a divergent local
                // reconstruction must reach NEITHER the signer engine NOR W1
                // (which would publish the wrong key into the shared map, the
                // root of the cross-epoch poisoning at soak v39). `Absent` ⇒ the
                // share-gate demotes to verify-only; the recompute-heal later
                // stores the correct exact-epoch key and re-promotes.
                if mint_diverges_from_attested(
                    &group_keys,
                    minted_at,
                    super::outcome::group_public_key(out),
                ) {
                    tracing::debug!(
                        epoch,
                        minted_at,
                        "carry-forward refused: local mint key diverges from the \
                         network-attested key (share-gate will demote to verify-only)"
                    );
                    metrics::counter!(
                        "dpos_carry_forward_refused_total",
                        "reason" => "key_divergence",
                        "path" => "share"
                    )
                    .increment(1);
                    return BeaconResolve::Absent;
                }
                BeaconResolve::Key((out.public().clone(), Some(share.clone()), namespace.clone()))
            }
            CarryVerdict::NoUsableMint => {
                // The chain's key epoch names a mint this node never attended,
                // or the node's own newer mint was chain-declined — no usable
                // share for E. Never hand it to the promote path.
                tracing::debug!(
                    epoch,
                    "carry-forward refused: this node holds no mint at the chain's \
                     dkgQual key epoch (share-gate will demote to verify-only)"
                );
                metrics::counter!(
                    "dpos_carry_forward_refused_total",
                    "reason" => "no_usable_mint",
                    "path" => "share"
                )
                .increment(1);
                BeaconResolve::Absent
            }
            CarryVerdict::ReadFailed => BeaconResolve::Absent,
        }
    })
}

#[cfg(test)]
mod carry_arbitration_tests {
    use super::beacon_share_resolver;
    use crate::beacon::{carry::DkgQualFor, ceremony::CeremonyOutput};
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{group::Share, sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1};
    use fluentbase_bls::PeerPubkey;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::{
        collections::BTreeMap,
        sync::{Arc, RwLock},
    };

    fn committee(seed: u64, n: usize) -> Set<PeerPubkey> {
        let mut rng = StdRng::seed_from_u64(seed);
        Set::from_iter_dedup((0..n).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()))
    }

    /// An empty group-key map: no network-attested (W4 ObservedOutcome) entries,
    /// so the resolvers' carry-divergence guard never fires — the pre-guard
    /// behavior these arbitration tests assert.
    fn no_attested() -> crate::beacon::keys::BeaconKeys {
        crate::beacon::keys::BeaconKeys::new()
    }

    /// A frozen on-chain `dkgQual` history: the given epochs have the bit set.
    fn qual(bits: &[u64]) -> DkgQualFor {
        let set: std::collections::BTreeSet<u64> = bits.iter().copied().collect();
        Arc::new(move |e| Some(set.contains(&e)))
    }

    /// A real committee DKG over `players`, as the `DkgActor` would memoize it.
    fn ceremony(players: &Set<PeerPubkey>) -> (CeremonyOutput, Share) {
        let mut rng = StdRng::seed_from_u64(0xD1);
        let (outcome, shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players.clone())
                .expect("deal");
        let share = shares
            .get_value(players.iter().next().expect("non-empty"))
            .expect("share")
            .clone();
        (outcome, share)
    }

    /// The share-gate resolver enforces the same arbitration end-to-end:
    /// no re-mint since the stored mint ⇒ the carried `(PK, share)`; a missed
    /// re-mint ⇒ `Absent` (⇒ share-gate demote ⇒ heal); an unreadable bit ⇒
    /// `Absent` (undecided — retried on the next resolve edge).
    #[test]
    fn share_resolver_refuses_stale_carry_and_undecided_bits() {
        use crate::beacon::surface::BeaconResolve;
        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let pk = *crate::beacon::outcome::group_public_key(&outcome);
        let store = Arc::new(RwLock::new(BTreeMap::from([(5u64, (outcome, share))])));

        // No re-mint since 5 ⇒ carried key confirmed.
        let resolve = crate::beacon::resolve::beacon_share_resolver(
            store.clone(),
            qual(&[5]),
            b"ns".to_vec(),
            no_attested(),
        );
        match resolve(12) {
            BeaconResolve::Key((sharing, share, ns)) => {
                assert_eq!(*sharing.public(), pk);
                assert!(share.is_some());
                assert_eq!(ns, b"ns");
            }
            _ => panic!("carry-forward must resolve the stored key"),
        }

        // A→B→A sandwich ⇒ Absent (pre-fix: the stale Key — the W1-poisoning root).
        let resolve = beacon_share_resolver(
            store.clone(),
            qual(&[5, 8, 11]),
            b"ns".to_vec(),
            no_attested(),
        );
        assert!(matches!(resolve(12), BeaconResolve::Absent));

        // Unreadable bit ⇒ Absent (undecided, retried).
        let holey: DkgQualFor = Arc::new(|e| (e != 7).then_some(e == 5));
        let resolve = beacon_share_resolver(store, holey, b"ns".to_vec(), no_attested());
        assert!(matches!(resolve(12), BeaconResolve::Absent));
    }

    /// The soak-v39 carry-DIVERGENCE guard. A member
    /// that finalized a DIVERGENT local outcome at the mint epoch (a torn /
    /// superset log set → a different `Logs::select` → a different `PK_E`) holds
    /// a self-derived key nobody else signs with. The no-mint span is `Confirmed`
    /// (the TIMING is right — `minted_at` IS the latest mint ≤ epoch), so the
    /// pre-fix resolvers served/promoted it → a lone `reject{bad_signature}` that
    /// split honest voters, plus a poisoned W1 map entry carried across stable
    /// epochs. The network-attested mint key (W4 ObservedOutcome) is the
    /// reference: on a DIFFERING value BOTH resolvers must refuse — the vote path
    /// to `Unknown` (accept-biased, no false reject), the share gate to `Absent`
    /// (demote to verify-only, never W1-publish) — and the refusal must carry to
    /// every stable epoch that inherits the mint (query 7, mint 5). A MATCHING
    /// attestation (the correctly-qualified restarted signer) stays trusted.
    #[test]
    fn divergent_carry_is_refused_against_the_attested_mint_key() {
        use crate::beacon::keys::KeySource;
        use crate::beacon::surface::BeaconResolve;

        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let local_pk = *crate::beacon::outcome::group_public_key(&outcome);
        // Our own material, minted at epoch 5 (a change epoch — bit set).
        let store = Arc::new(RwLock::new(BTreeMap::from([(5u64, (outcome, share))])));

        // A DIFFERENT network-attested key for the SAME mint epoch 5 (W4). The
        // shared `ceremony` helper deals under a fixed seed, so a distinct PK_E
        // needs a distinct deal seed here.
        let attested_pk = {
            let mut rng = StdRng::seed_from_u64(0xBEEF);
            let (other_out, _) = deal::<MinSig, PeerPubkey, N3f1>(
                &mut rng,
                Mode::NonZeroCounter,
                committee(0xEE, 4),
            )
            .expect("deal");
            *crate::beacon::outcome::group_public_key(&other_out)
        };
        assert_ne!(attested_pk, local_pk, "test needs a genuine divergence");
        let diverged = crate::beacon::keys::BeaconKeys::new();
        diverged.set_pk(5, attested_pk, KeySource::Agreed);

        // Share gate — carry to STABLE epoch 7 ⇒ Absent (share-gate demotes;
        // never W1-publishes).
        let sign = beacon_share_resolver(store.clone(), qual(&[5]), b"ns".to_vec(), diverged);
        assert!(
            matches!(sign(7), BeaconResolve::Absent),
            "a divergent carried key must not promote / W1-publish"
        );

        // A MATCHING attestation (own == network) leaves the key trusted — the
        // correctly-qualified restarted signer is NOT over-blocked.
        let agreeing = crate::beacon::keys::BeaconKeys::new();
        agreeing.set_pk(5, local_pk, KeySource::Agreed);
        match beacon_share_resolver(store, qual(&[5]), b"ns".to_vec(), agreeing)(7) {
            BeaconResolve::Key((sharing, _, _)) => assert_eq!(*sharing.public(), local_pk),
            _ => panic!("a network-corroborated own key stays trusted"),
        }
    }

    /// The tripwire above is keyed on the MINT, and on a committee that never
    /// changes the mint is the deterministic bootstrap epoch forever, while the
    /// pruners (`epoch_manager`'s reconcile and the cert-inlet's per-cert sweep)
    /// run frontier-relative. An epoch-measured window over the whole store
    /// therefore deletes the one entry the guard reads once the frontier passes
    /// `mint + SCHEME_RETENTION_EPOCHS`, and nothing re-inserts it: the only
    /// `Agreed` producers at a mint epoch are the agreement write-back (a
    /// committee CHANGE only) and the ladder's own memoisation, neither of which a
    /// plain validator reaches on a healthy stable chain. Both resolvers would
    /// then serve the divergent local material they refuse above.
    ///
    /// Reds if `retain_from` stops exempting [`KeySource::Agreed`].
    #[test]
    fn a_stable_committees_attested_mint_outlives_the_retention_window() {
        use crate::beacon::keys::KeySource;
        use crate::beacon::surface::BeaconResolve;

        let bootstrap = crate::beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let local_pk = *crate::beacon::outcome::group_public_key(&outcome);
        let store = Arc::new(RwLock::new(BTreeMap::from([(bootstrap, (outcome, share))])));

        let attested_pk = {
            let mut rng = StdRng::seed_from_u64(0xF00D);
            let (other_out, _) = deal::<MinSig, PeerPubkey, N3f1>(
                &mut rng,
                Mode::NonZeroCounter,
                committee(0xEE, 4),
            )
            .expect("deal");
            *crate::beacon::outcome::group_public_key(&other_out)
        };
        assert_ne!(attested_pk, local_pk, "test needs a genuine divergence");
        let diverged = crate::beacon::keys::BeaconKeys::new();
        diverged.set_pk(bootstrap, attested_pk, KeySource::Agreed);

        // A W1 publication and a carry memo for a long-past epoch: the derived
        // tiers the window exists to bound, so the prune must still take them.
        diverged.set_pk(bootstrap + 1, local_pk, KeySource::LocalDkg);
        diverged.set_pk(bootstrap + 2, local_pk, KeySource::Carried);

        // The frontier walks well past `bootstrap + SCHEME_RETENTION_EPOCHS`, one
        // prune per epoch entered, exactly as both live callers do.
        let frontier = bootstrap + 10 * crate::SCHEME_RETENTION_EPOCHS as u64;
        for epoch in bootstrap..=frontier {
            diverged.retain_from(epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64));
        }
        assert_eq!(
            diverged.cached_only(bootstrap + 1),
            None,
            "a local publication far below the frontier is still pruned"
        );
        assert_eq!(
            diverged.cached_only(bootstrap + 2),
            None,
            "and so is a carry memo"
        );

        // A stable epoch at the far frontier still carries the bootstrap mint, so
        // the tripwire must still see the attestation and refuse.
        let sign = beacon_share_resolver(store, qual(&[]), b"ns".to_vec(), diverged);
        assert!(
            matches!(sign(frontier), BeaconResolve::Absent),
            "and the share gate must keep demoting rather than W1-publish a divergent key"
        );
    }

    /// Defect 2 (soak v47, epoch 4→5): at a CHANGE boundary the local candidate
    /// ceremony COMPLETED (players = candidate committee) but its DKG
    /// under-qualified on-chain — `dkgQual[5]` never landed, so the contract
    /// re-committed the incumbent. The store holds BOTH mints. The declined
    /// candidate mint (bit clear) must be UNUSED and the incumbent's carried
    /// key (bit set at 3) served; the pre-fix code refused → every seated node
    /// demoted → zero proposers → freeze. Both resolvers must serve the
    /// incumbent.
    #[test]
    fn declined_candidate_falls_back_to_committed_incumbent() {
        use crate::beacon::surface::BeaconResolve;

        let incumbent = committee(0xC0, 4);
        let candidate = committee(0xCA, 4);
        let (inc_out, inc_share) = ceremony(&incumbent);
        let (cand_out, cand_share) = ceremony(&candidate);
        let inc_pk = *crate::beacon::outcome::group_public_key(&inc_out);

        let store = Arc::new(RwLock::new(BTreeMap::from([
            (3u64, (inc_out, inc_share)),
            (5u64, (cand_out, cand_share)),
        ])));

        let sign = beacon_share_resolver(store, qual(&[3]), b"ns".to_vec(), no_attested());
        match sign(5) {
            BeaconResolve::Key((sharing, share, _)) => {
                assert_eq!(*sharing.public(), inc_pk);
                assert!(share.is_some());
            }
            _ => panic!(
                "the declined candidate mint must be skipped and the committed \
                 incumbent's key served, not a demote"
            ),
        }
    }
}
