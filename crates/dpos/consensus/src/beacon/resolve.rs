//! Resolving `PK_epoch` from THIS node's own material, and checking a block's
//! parent-seed witness under it.
//!
//! Everything here used to be split across the consensus core: the 3-state
//! lookup and the witness check sat in `application.rs`, and the two closures
//! that actually read the ceremony store were built at the launch site in
//! `dpos.rs`. They are one decision — "does this node hold the epoch's group
//! key, and does this signature verify under it" — and they answer it out of
//! DKG material, carry-forward arbitration and an agreement store, none of
//! which the core is allowed to know exists.
//!
//! The core sees only [`super::WitnessCheck`], through
//! [`super::Randomness::check_witness`].

use super::{
    carry::{select_carry_scheme, CarryVerdict, DkgQualFor},
    keys::{pk_prefix, BeaconKeys, KeySource},
    seed::Seed,
    BeaconResolve, BeaconResolver, WitnessCheck,
};
use crate::beacon::actor::CeremonyStore;
use fluentbase_bls::beacon::{verify_seed, GroupPublic};
use std::sync::Arc;

/// Outcome of a group-key resolution (`PK_epoch`), 3-state. Conflating the
/// last two states IS the P1 bug: `Unknown` = "this node structurally does not
/// hold `PK_epoch`" (a stable fact about this node — re-polling cannot help);
/// `ReadFailed` = "the committee read was transiently unavailable, I could not
/// even decide" (retried, NEVER cached). The 2-state `BeaconResolver` fold
/// (`_ => None`, `dpos.rs`) is fine for its retryable share-gate consumer but
/// unusable on a vote path — do not collapse this enum into it.
// A `GroupPublic` (G2) is ~288 B; the enum is a transient return value that is
// matched immediately and never stored, so the stack copy is cheaper than the
// per-resolve heap allocation boxing would put on the vote path.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyLookup {
    Resolved(GroupPublic),
    Unknown,
    ReadFailed,
}

/// The lazy 3-state group-key resolver (ladder step 1: the node's OWN live-DKG
/// material, carry-forward + committee-equality gated). Built at the launch
/// site (`dpos.rs`) — the only place the `CeremonyStore` exists — and threaded
/// into [`BeaconVerify`]. Reads `.public()` ONLY, never the share.
pub(crate) type GroupKeyFor = Arc<dyn Fn(u64) -> KeyLookup + Send + Sync>;

/// The per-epoch beacon context threaded into [`FluentApp`]'s verify path.
///
/// Since the epoch key left `OrderBlock`, a block asserts nothing about the
/// beacon and there is no boundary gate: what remains is the two things the
/// parent-seed witness needs — a `PK_epoch` resolver and the seed-signing
/// domain. `None` on `FluentApp` ⇒ no beacon context (cold-start epoch 0 /
/// followers / tests) ⇒ [`FluentApp::group_public_for`] answers `Unknown` and the
/// witness arm takes its accept-biased branch.
#[derive(Clone)]
pub struct BeaconVerify {
    /// Lazy 3-state `PK_epoch` resolver (ladder step 1) — consulted by
    /// [`FluentApp::group_public_for`] on a [`BeaconKeys`] store miss; the map
    /// memoizes `Resolved` only. Carry-forward + committee-gated, key-only.
    group_key_for: GroupKeyFor,
    /// The chain's beacon seed-signing namespace
    /// (`seed_namespace(fluent_namespace(chain_id))`) — the domain the witness
    /// signature is verified under (`verify_seed`).
    seed_namespace: Vec<u8>,
}

impl BeaconVerify {
    pub fn new(group_key_for: GroupKeyFor, seed_namespace: Vec<u8>) -> Self {
        Self {
            group_key_for,
            seed_namespace,
        }
    }
}

/// Resolve `PK_epoch` for the witness-signature arm: the shared [`BeaconKeys`]
/// store first (the common case — no I/O; every voting node finds its OWN epoch
/// there via W1), then the lazy 3-state resolver (ladder step 1), memoizing ONLY
/// on `Resolved`. `Unknown` and `ReadFailed` are NEVER cached — a later call
/// re-runs the resolve and can succeed (the DKG actor writes the `CeremonyStore`
/// asynchronously, and W4 can land the key from an observed outcome block at any
/// time). Synchronous by construction: both inputs are in-memory (the map and
/// the resolver's `CeremonyStore`/committee snapshot) — no await, no in-flight
/// state on the vote path.
pub(crate) fn resolve_group_public(
    group_keys: &BeaconKeys,
    beacon: Option<&BeaconVerify>,
    epoch: u64,
) -> KeyLookup {
    if let Some(pk) = group_keys.cached_only(epoch) {
        metrics::counter!("dpos_group_public_source_total", "ladder" => "map").increment(1);
        return KeyLookup::Resolved(pk);
    }
    let Some(bv) = beacon else {
        // No beacon context (follower / test) ⇒ this node structurally
        // holds no DKG material — a stable fact, not a transient.
        return KeyLookup::Unknown;
    };
    match (bv.group_key_for)(epoch) {
        KeyLookup::Resolved(pk) => {
            metrics::counter!("dpos_group_public_source_total", "ladder" => "dkg").increment(1);
            tracing::debug!(
                epoch,
                group_public = %pk_prefix(&pk),
                "group key resolved from own DKG material (ladder=dkg); memoizing"
            );
            group_keys.set_pk(epoch, pk, KeySource::LocalDkg);
            KeyLookup::Resolved(pk)
        }
        miss => miss,
    }
}

/// Resolve `PK_Ep` and check `seed` under it — the witness arm's WHOLE decision,
/// as one unit.
///
/// Free fn over its inputs rather than a method: the resolve and the check are a
/// single decision, and having them live in a method plus an inline match at the
/// call site is what made that invisible. The vote mapping, the
/// `dpos_parent_seed_*` counters and the same-epoch-miss assert stay with the
/// caller — they describe how THIS node votes, not how the key resolves.
pub(crate) fn resolve_witness(
    group_keys: &BeaconKeys,
    beacon: Option<&BeaconVerify>,
    parent_epoch: u64,
    seed: &Seed,
) -> WitnessCheck {
    // NO CONTEXT ⇒ NO VERDICT, and this must come BEFORE the resolve. The
    // namespace a witness signature is verified under lives inside `BeaconVerify`;
    // with no context there is no namespace, and the code below used to fall back
    // to an EMPTY one. That is not a weaker check, it is a different message:
    // `union_unique` length-prefixes the namespace, so an empty one verifies a
    // signature that was never produced — every VALID witness comes back
    // `Invalid`, and this node casts a reject vote on a key it never actually
    // checked.
    //
    // `resolve_group_public` reads the shared store BEFORE it looks at `beacon`,
    // so a provider with no context but a non-empty store reaches exactly that.
    // No production wiring produces that pair today — the app's provider is either
    // the full plane one or the permanently-negative one — but it held by ASSEMBLY,
    // not by structure, and the one constructor that could produce it advertised
    // itself as a production entry point. Refusing here makes the state
    // unrepresentable instead of merely unreached.
    let Some(bv) = beacon else {
        return WitnessCheck::NoKey;
    };
    match resolve_group_public(group_keys, Some(bv), parent_epoch) {
        KeyLookup::Resolved(pk) => {
            let ns = bv.seed_namespace.as_slice();
            if verify_seed(&pk, ns, seed.target_round, &seed.signature) {
                return WitnessCheck::Valid;
            }
            // Loud + byte-diffable: the resolved-key fingerprint is what lets a
            // lone rejecting node's PK_Ep be compared against the quorum's from
            // logs alone (a diverged carried-forward key rejects here with the
            // ladder never having run — soak 2026-07-14 v5@epoch77). It is
            // emitted HERE and not at the call site because the caller does not
            // hold the key.
            tracing::warn!(
                parent_epoch,
                round = ?seed.target_round,
                group_public = %pk_prefix(&pk),
                "parent-seed witness FAILED signature verify under resolved PK_Ep"
            );
            WitnessCheck::Invalid
        }
        KeyLookup::Unknown => WitnessCheck::NoKey,
        KeyLookup::ReadFailed => WitnessCheck::Undecided,
    }
}

/// Build the lazy 3-state group-key resolver (§5 b, ladder step 1): resolve
/// `PK_epoch` from the node's OWN live-DKG material under ON-CHAIN
/// `dkgQual`-BIT ARBITRATION (`beacon::carry::select_carry_scheme`): the chain's
/// key epoch for E is the last set bit in `(BOOTSTRAP, E]` (else the bootstrap
/// mint), and the node serves its stored mint at exactly that epoch — a newer
/// local mint the chain DECLINED (bit clear — soak v47) is UNUSED, a re-mint
/// missed during downtime (departure-then-backfill, soak 2026-07-14) is a set
/// bit the store misses ⇒ `Unknown`. A stable committee re-uses the last change
/// epoch's key and writes no new `CeremonyStore` entry, so an exact `get(&E)`
/// misses on every stable epoch — the bit scan IS the carry. Reads `.public()`
/// ONLY, never the share. The 3 states are load-bearing (P1): `Unknown` = this
/// node structurally holds no `PK_epoch` (incl. a chain-declined or superseded
/// mint); `ReadFailed` = could not decide (a `dkgQual` read failed) — retried
/// by the caller, NEVER cached, and accept-biased on the witness vote arm, so
/// an undecided resolve never turns into a false reject. Do not fold them (that
/// is `beacon_resolver`'s `Absent` arm, unusable on a vote path).
///
/// Survives a restart: the ceremony store reloads from `<datadir>/beacon/` at
/// plane startup, so a restarted signer on a stable committee resolves its own
/// epoch's key here even when the observed-outcome cursor is empty and the
/// 8-hop marshal walk is exhausted (the R1 rolling-restart halt).
pub(crate) fn group_key_resolver(
    store: CeremonyStore,
    dkg_qual: DkgQualFor,
    group_keys: BeaconKeys,
) -> GroupKeyFor {
    Arc::new(move |epoch: u64| {
        let Ok(m) = store.read() else {
            // Poisoned store lock: "could not decide" — transient-shaped, so
            // it must not be conflated with a structural miss.
            return KeyLookup::ReadFailed;
        };
        match select_carry_scheme(epoch, |e| m.contains_key(&e), &dkg_qual) {
            CarryVerdict::Serve { minted_at } => {
                let (out, _share) = m.get(&minted_at).expect("select returned a stored mint");
                let pk = *super::outcome::group_public_key(out);
                // CARRY-DIVERGENCE guard (mirror of the epoch_manager promote
                // VALUE-gate, on the VOTE path): the on-chain arbitration proves
                // only that the chain minted AT `minted_at`, NOT that this
                // node's LOCAL outcome at that mint matches the chain's. A
                // member that finalized a divergent outcome (a torn/superset log
                // set → a different `Logs::select` → a different `PK_E`; see
                // `recompute_scoped`) holds a self-derived key nobody signs with.
                // Serving it here drives a lone `reject{bad_signature}` that
                // splits honest voters (soak v39). The
                // network-attested mint key (W4 ObservedOutcome — agreed chain
                // data) is the reference: on a DIFFERING value the carried
                // material is untrusted for verification ⇒ `Unknown`
                // (accept-biased), NEVER `Resolved`. Keyed on `minted_at` (the
                // mint), so ONE observed outcome demotes the mint AND every
                // stable epoch that carries it. Absent attestation ⇒ unchanged
                // (the restarted-signer carry).
                if let Some(net) = group_keys.attested(minted_at) {
                    if net != pk {
                        metrics::counter!(
                            "dpos_carry_forward_refused_total",
                            "reason" => "key_divergence",
                            "path" => "verify"
                        )
                        .increment(1);
                        return KeyLookup::Unknown;
                    }
                }
                KeyLookup::Resolved(pk)
            }
            // The node holds no mint at the chain's key epoch (never attended
            // it, or its own newer mint was chain-declined): no usable key —
            // a stable fact about this node, not a transient.
            CarryVerdict::NoUsableMint => KeyLookup::Unknown,
            CarryVerdict::ReadFailed => KeyLookup::ReadFailed,
        }
    })
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
                // SHARE-GATE carry-divergence guard (same primitive as the
                // vote-path `group_key_resolver`): on-chain arbitration proves
                // only that the chain minted AT `minted_at`, NOT that
                // our LOCAL outcome matches the chain's. When the network-attested
                // mint key (W4 ObservedOutcome) DIFFERS from our self-derived
                // one, the carried `(PK_E, share)` is a divergent local
                // reconstruction — hand it to NEITHER the signer engine NOR W1
                // (which would publish the wrong key into the shared map, the
                // root of the cross-epoch poisoning at soak v39). `Absent` ⇒ the
                // share-gate demotes to verify-only; the recompute-heal later
                // stores the correct exact-epoch key and re-promotes.
                let (out, share) = m.get(&minted_at).expect("select returned a stored mint");
                let pk = out.public().clone();
                let pk_g2 = *super::outcome::group_public_key(out);
                if let Some(net) = group_keys.attested(minted_at) {
                    if net != pk_g2 {
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
                }
                BeaconResolve::Key((pk, Some(share.clone()), namespace.clone()))
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
mod group_key_resolver_tests {
    use super::{beacon_share_resolver, group_key_resolver};
    use crate::beacon::{carry::DkgQualFor, ceremony::CeremonyOutput, KeyLookup};
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

    /// (R1) — the rolling-restart chain-halt regression, at the resolver level.
    /// A restarted signer of a STABLE committee holds its DKG material keyed at
    /// the last CHANGE epoch (`Ec − 9` — a stable epoch writes no new
    /// `CeremonyStore` entry) and its attested-key store entry is EMPTY (a
    /// stable committee runs no agreement, so nothing is keyed at `Ec`).
    /// Ladder step 1 (carry-forward + committee equality) MUST still
    /// resolve `PK_Ec` — under the pre-R1 spec (cursor + walk only) this is
    /// `None`, the node votes `false` on every honest block, and `f+1` such
    /// nodes are a permanent self-sustaining halt (the only event that would
    /// repopulate the cursor — a change epoch's outcome block — can never be
    /// produced by a halted chain).
    #[test]
    fn restarted_signer_on_stable_committee_resolves_pk_via_carry_forward() {
        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let pk = *crate::beacon::outcome::group_public_key(&outcome);

        // Material keyed at the last change epoch, NOT at the queried epoch.
        let change_epoch = 3u64;
        let ec = change_epoch + 9;
        let store = Arc::new(RwLock::new(BTreeMap::from([(
            change_epoch,
            (outcome, share),
        )])));

        // The chain minted at the change epoch and never re-minted since.
        let resolve = group_key_resolver(store, qual(&[change_epoch]), no_attested());

        assert_eq!(
            resolve(ec),
            KeyLookup::Resolved(pk),
            "own DKG material must carry forward across stable epochs"
        );
    }

    /// The 3 states are distinguishable — conflating the last two IS the P1
    /// bug (`Unknown` = structurally no key; `ReadFailed` = transiently
    /// undecidable, retried, never cached).
    #[test]
    fn resolver_distinguishes_unknown_from_read_failed() {
        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let store = Arc::new(RwLock::new(BTreeMap::from([(3u64, (outcome, share))])));

        // Transiently unreadable dkgQual bits ⇒ ReadFailed (NOT Unknown).
        let unreadable: DkgQualFor = Arc::new(|_| None);
        assert_eq!(
            crate::beacon::resolve::group_key_resolver(store.clone(), unreadable, no_attested())(
                12
            ),
            KeyLookup::ReadFailed
        );

        // The chain re-minted at 8 and this node holds no mint there ⇒ Unknown.
        assert_eq!(
            group_key_resolver(store, qual(&[3, 8]), no_attested())(12),
            KeyLookup::Unknown
        );

        // No ceremony material at or below the epoch ⇒ Unknown.
        let empty = Arc::new(RwLock::new(BTreeMap::new()));
        assert_eq!(
            group_key_resolver(empty, qual(&[]), no_attested())(12),
            KeyLookup::Unknown
        );
    }

    /// A→B→A committee sandwich across missed re-mints, at the VOTE-PATH
    /// resolver: the store holds ceremony(5), but the chain's `dkgQual` bits
    /// record re-mints at 8 and 11 this node never attended. The pre-fix
    /// players-equality guard resolved the STALE key here (⇒ a false
    /// `reject{bad_signature}` on a valid witness — soak 2026-07-14
    /// v5@epoch77); the bit arbitration must return `Unknown` (structural —
    /// our material is an older key).
    #[test]
    fn stale_key_across_a_missed_remint_is_unknown_not_resolved() {
        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let store = Arc::new(RwLock::new(BTreeMap::from([(5u64, (outcome, share))])));

        assert_eq!(
            group_key_resolver(store, qual(&[5, 8, 11]), no_attested())(12),
            KeyLookup::Unknown
        );
    }

    /// An unreadable `dkgQual` bit is UNDECIDED — surfaced as the transient
    /// `ReadFailed` (accept-biased on the witness arm, retried, never cached),
    /// NOT as a resolved stale key.
    #[test]
    fn unreadable_bit_is_read_failed() {
        let players = committee(0xC0, 4);
        let (outcome, share) = ceremony(&players);
        let store = Arc::new(RwLock::new(BTreeMap::from([(5u64, (outcome, share))])));

        let holey: DkgQualFor = Arc::new(|e| (e != 7).then_some(e == 5));
        assert_eq!(
            group_key_resolver(store, holey, no_attested())(12),
            KeyLookup::ReadFailed
        );
    }

    /// The share-gate resolver enforces the same arbitration end-to-end:
    /// no re-mint since the stored mint ⇒ the carried `(PK, share)`; a missed
    /// re-mint ⇒ `Absent` (⇒ share-gate demote ⇒ heal); an unreadable bit ⇒
    /// `Absent` (undecided — retried on the next resolve edge).
    #[test]
    fn share_resolver_refuses_stale_carry_and_undecided_bits() {
        use crate::beacon::BeaconResolve;
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
        use crate::beacon::BeaconResolve;

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

        // Vote path — carry to STABLE epoch 7 ⇒ Unknown (accept-biased), never Resolved.
        let verify = group_key_resolver(store.clone(), qual(&[5]), diverged.clone());
        assert_eq!(
            verify(7),
            KeyLookup::Unknown,
            "a divergent carried key must not be served for verification"
        );

        // Share gate — same input ⇒ Absent (share-gate demotes; never W1-publishes).
        let sign = beacon_share_resolver(store.clone(), qual(&[5]), b"ns".to_vec(), diverged);
        assert!(
            matches!(sign(7), BeaconResolve::Absent),
            "a divergent carried key must not promote / W1-publish"
        );

        // A MATCHING attestation (own == network) leaves the key trusted — the
        // correctly-qualified restarted signer is NOT over-blocked.
        let agreeing = crate::beacon::keys::BeaconKeys::new();
        agreeing.set_pk(5, local_pk, KeySource::Agreed);
        let verify_ok = group_key_resolver(store, qual(&[5]), agreeing);
        assert_eq!(
            verify_ok(7),
            KeyLookup::Resolved(local_pk),
            "a network-corroborated own key stays trusted"
        );
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
        use crate::beacon::BeaconResolve;

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
        // both tripwires must still see the attestation and refuse.
        let verify = group_key_resolver(store.clone(), qual(&[]), diverged.clone());
        assert_eq!(
            verify(frontier),
            KeyLookup::Unknown,
            "the divergence guard must stay armed for the life of a stable committee"
        );
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
        use crate::beacon::BeaconResolve;

        let incumbent = committee(0xC0, 4);
        let candidate = committee(0xCA, 4);
        let (inc_out, inc_share) = ceremony(&incumbent);
        let (cand_out, cand_share) = ceremony(&candidate);
        let inc_pk = *crate::beacon::outcome::group_public_key(&inc_out);

        let store = Arc::new(RwLock::new(BTreeMap::from([
            (3u64, (inc_out, inc_share)),
            (5u64, (cand_out, cand_share)),
        ])));

        let verify = group_key_resolver(store.clone(), qual(&[3]), no_attested());
        assert_eq!(
            verify(5),
            KeyLookup::Resolved(inc_pk),
            "the declined candidate mint is skipped; the committed incumbent's key is served"
        );

        let sign = beacon_share_resolver(store, qual(&[3]), b"ns".to_vec(), no_attested());
        match sign(5) {
            BeaconResolve::Key((sharing, share, _)) => {
                assert_eq!(*sharing.public(), inc_pk);
                assert!(share.is_some());
            }
            _ => panic!("share gate must serve the committed incumbent, not demote"),
        }
    }
}

#[cfg(test)]
mod ladder_tests {
    use super::*;
    use crate::beacon::keys::KeySource;
    use fluentbase_bls::beacon::GroupPublic;
    use std::sync::{atomic::Ordering, Arc};

    fn test_group_keys() -> BeaconKeys {
        BeaconKeys::new()
    }

    /// The group-key LADDER under test (P1): the shared store first, then the
    /// 3-state resolver, memoizing only on `Resolved`.
    ///
    /// These used to be driven through a whole `FluentApp` via
    /// `FluentApp::group_public_for`. That method is gone — the app reads the
    /// witness verdict off the randomness surface now and never resolves a key
    /// itself — so the fixture is the ladder itself, which is what the tests
    /// were always about.
    struct Ladder {
        keys: BeaconKeys,
        verify: BeaconVerify,
    }

    impl Ladder {
        fn new(keys: BeaconKeys, group_key_for: GroupKeyFor) -> Self {
            Self {
                verify: BeaconVerify::new(group_key_for, Vec::new()),
                keys,
            }
        }

        fn group_public_for(&self, epoch: u64) -> KeyLookup {
            resolve_group_public(&self.keys, Some(&self.verify), epoch)
        }
    }

    /// A real `PK_epoch` value for the group-key fixtures.
    fn sample_group_public() -> GroupPublic {
        use commonware_cryptography::bls12381::dkg::deal_anonymous;
        use commonware_utils::{test_rng, N3f1, NZU32};
        let mut rng = test_rng();
        let (sharing, _shares) = deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            N3f1,
        >(&mut rng, Default::default(), NZU32!(4));
        *sharing.public()
    }

    /// (P1-a) — the sticky-`None` regression, at the resolver/map level: a
    /// TRANSIENT `committee_for` outage must produce `ReadFailed`, cache
    /// NOTHING (no entry of any kind — a one-shot-at-spawn resolution would
    /// make it sticky for the whole epoch, and staking reads fail correlated
    /// across validators ⇒ `f+1` in the accept-arm set ⇒ the boundary forge
    /// arm), and RESOLVE on a later call once the outage clears — memoizing
    /// only then.
    #[test]
    fn a_transient_committee_read_failure_does_not_become_a_sticky_epoch_wide_none() {
        use std::sync::atomic::{AtomicBool, AtomicU32};

        let pk = sample_group_public();
        let calls = Arc::new(AtomicU32::new(0));
        let outage = Arc::new(AtomicBool::new(true));
        let (c, o) = (calls.clone(), outage.clone());
        let resolver: GroupKeyFor = Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            if o.load(Ordering::SeqCst) {
                KeyLookup::ReadFailed
            } else {
                KeyLookup::Resolved(pk)
            }
        });
        let group_keys = test_group_keys();
        let app = Ladder::new(group_keys.clone(), resolver);

        // During the outage: ReadFailed, and the failure is NOT cached.
        assert_eq!(app.group_public_for(7), KeyLookup::ReadFailed);
        assert!(
            group_keys.cached_only(7).is_none(),
            "a failure must never be inserted into the map"
        );

        // Outage clears ⇒ the SAME call path resolves (nothing negative was
        // memoized) and the success is cached.
        outage.store(false, Ordering::SeqCst);
        assert_eq!(app.group_public_for(7), KeyLookup::Resolved(pk));
        assert_eq!(group_keys.cached_only(7), Some(pk));
        assert_eq!(group_keys.attested(7), None, "a W1 memoize is local-tier");

        // Subsequent reads hit the map — the resolver is not consulted again.
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(app.group_public_for(7), KeyLookup::Resolved(pk));
        assert_eq!(calls.load(Ordering::SeqCst), before, "map hit is I/O-free");
    }

    /// `Unknown` is a stable fact and is NEVER cached either — a later call
    /// re-runs the resolve and can succeed (the DKG store is written
    /// asynchronously; W4 can land the key from a block at any time).
    #[test]
    fn unknown_is_not_cached_and_can_become_resolved() {
        use std::sync::atomic::AtomicBool;

        let pk = sample_group_public();
        let has_material = Arc::new(AtomicBool::new(false));
        let h = has_material.clone();
        let resolver: GroupKeyFor = Arc::new(move |_| {
            if h.load(Ordering::SeqCst) {
                KeyLookup::Resolved(pk)
            } else {
                KeyLookup::Unknown
            }
        });
        let group_keys = test_group_keys();
        let app = Ladder::new(group_keys.clone(), resolver);

        assert_eq!(app.group_public_for(9), KeyLookup::Unknown);
        assert!(group_keys.is_empty());

        has_material.store(true, Ordering::SeqCst);
        assert_eq!(app.group_public_for(9), KeyLookup::Resolved(pk));
        assert_eq!(group_keys.cached_only(9), Some(pk));
        assert_eq!(group_keys.attested(9), None, "a W1 memoize is local-tier");
    }

    /// W1/W2 — a map entry written at engine spawn is read with ZERO resolver
    /// calls: a continuing member performs no `committee_for` read at the next
    /// boundary, so a correlated staking-read outage cannot move it into the
    /// accept-arm set.
    #[test]
    fn a_pre_populated_map_entry_never_touches_the_resolver() {
        let pk = sample_group_public();
        let resolver: GroupKeyFor = Arc::new(move |_| {
            panic!("the resolver must not run on a map hit");
        });
        let group_keys = test_group_keys();
        group_keys.set_pk(4, pk, KeySource::LocalDkg); // as W1 does, before the engine
        let app = Ladder::new(group_keys, resolver);

        assert_eq!(app.group_public_for(4), KeyLookup::Resolved(pk));
    }
}
