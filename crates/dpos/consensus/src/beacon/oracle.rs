//! The beacon's synchronous face for the consensus scheme: the one place the
//! per-epoch threshold material is read on the vote path.
//!
//! The scheme holds no key material and asks this type instead, so the share and
//! the public polynomial never leave the beacon. Every answer is derived from
//! live shared state — the ceremony store the `DkgActor` writes and the group-key
//! store — rather than from a snapshot taken when the scheme was built, which is
//! what lets a scheme built before its epoch's ceremony finished start answering
//! the moment the entry lands.
//!
//! This type ANSWERS; it never decides participation. The demote decision belongs
//! to `share_probe`, which runs per reconcile.
//!
//! # There is no divergence gate here any more (П-3)
//!
//! It used to compare the polynomial it was about to judge with against the key a
//! `committee[minted_at]` quorum had attested, because the two came from DIFFERENT
//! objects: the polynomial from this node's own `CeremonyStore` entry, the attested
//! key from `BeaconKeys`. They can no longer differ — [`KeyIndex`] reads BOTH out of
//! the one artifact the mint's quorum certified, so "my polynomial disagrees with
//! the network's key" is not a state this type can be in. The gate, its
//! `dpos_seed_material_refused_divergent_total` counter and the soak-v39 class they
//! watched for are gone with the second owner that created them.
//!
//! The families this type still owns are read as edges and rates:
//! `dpos_seed_verify_{ok,no_key}_total`, whose split is what makes the keyless
//! window visible.

use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use commonware_consensus::types::Round;
use commonware_cryptography::bls12381::primitives::{
    group::Share,
    sharing::Sharing,
    variant::{MinSig, PartialSignature},
};
use commonware_utils::{N3f1, Participant};
use fluentbase_bls::{
    beacon,
    oracle::{SeedCheck, SeedOracle},
    BlsSignature,
};
use tracing::{error, warn};

use super::{actor::CeremonyStore, artifact::KeyIndex, metrics::BeaconMetrics};

/// The beacon-backed [`SeedOracle`] for ONE epoch.
///
/// Reads only synchronous state — [`KeyIndex`] (the chain's mint record plus the
/// artifact store) and this node's own share. Acquisition happens out of band: a
/// miss here IS the [`SeedCheck::NoKey`] answer, never a reason to go fetch on a
/// vote path.
#[derive(Clone)]
pub(crate) struct BeaconOracle {
    /// The epoch this oracle answers for — the TARGET of the carry arbitration,
    /// not necessarily the epoch its material was minted at. Its callers check
    /// the round's epoch binding before delegating, so nothing here re-derives it
    /// from a `Round`.
    pub(crate) epoch: u64,
    /// This node's own share per MINTING epoch. The polynomial it pairs with comes
    /// from [`Self::keys`], keyed by the same minting epoch — one object, so the
    /// pair cannot disagree.
    pub(crate) ceremony: CeremonyStore,
    /// The OWNER of `PK_epoch` and the public polynomial (П-3).
    pub(crate) keys: KeyIndex,
    /// The chain's beacon seed-signing namespace.
    pub(crate) namespace: Vec<u8>,
    /// This node's consensus participant index for the epoch, `None` for a
    /// verifier-flavored oracle. The share's own index must equal it or the
    /// partial is mis-attributed and recovers to nothing.
    pub(crate) me: Option<Participant>,
    /// First-refusal latch for the vote-quorum/seed-threshold mismatch in
    /// [`SeedOracle::recover`]. Construct it holding `false`. Separate from the
    /// seat latch below because the two conditions are independent and either
    /// alone must still be able to speak.
    pub(crate) warned_threshold_mismatch: Arc<AtomicBool>,
    /// First-refusal latch for the seat/share-index mismatch in
    /// [`SeedOracle::sign_partial`]. Construct it holding `false`.
    ///
    /// The condition is PERMANENT for the epoch — a share's index and this
    /// node's seat are both frozen at the boundary — but the check sits on the
    /// per-vote path, where the `assert_eq!` it replaced sat once per scheme
    /// construction. Without the latch one misconfigured epoch emits a warn per
    /// subject per view for the epoch's life, which is the shape this module's
    /// docs forbid for the resolver's counter and forbid here for the same
    /// reason.
    pub(crate) warned_seat_mismatch: Arc<AtomicBool>,
    /// The beacon's counters. Restored here by FLU-1202's Phase 3 together with
    /// the three families it bumps — Phase 2 dropped the field deliberately
    /// rather than carry one with nothing to increment.
    pub(crate) metrics: BeaconMetrics,
}

impl BeaconOracle {
    /// The polynomial and share this node may judge [`Self::epoch`] with.
    ///
    /// A stable committee writes no new `CeremonyStore` entry, so the material for
    /// `epoch` is keyed at the last CHANGE epoch and an exact `get(&epoch)` misses
    /// on every carried epoch — the mint lookup IS the lookup, not an enrichment of
    /// it, and [`KeyIndex`] is what does it (memoised, durable, one step).
    ///
    /// The polynomial comes from the ARTIFACT and the share from this node's own
    /// store, both keyed by the SAME minting epoch. That is the whole of what used
    /// to need a divergence gate: a share that does not lie on the artifact's
    /// polynomial is refused where it is ADOPTED (`DkgActor::adopt_share`'s
    /// `validate_share_on_poly`, П-3), so nothing that reaches here can disagree
    /// with itself.
    ///
    /// A poisoned lock degrades to a miss rather than propagating a panic onto the
    /// vote path.
    fn with_material<T>(&self, f: impl FnOnce(&Sharing<MinSig>, &Share) -> T) -> Option<T> {
        let (minted_at, sharing) = self.keys.sharing_at(self.epoch)?;
        let held = self.ceremony.read().ok()?;
        let share = held.get(&minted_at)?;
        Some(f(&sharing, share))
    }
}

impl SeedOracle for BeaconOracle {
    fn sign_partial(&self, round: Round) -> Option<BlsSignature> {
        self.with_material(|_, share| {
            if self.me != Some(share.index) {
                if !self.warned_seat_mismatch.swap(true, Ordering::Relaxed) {
                    warn!(
                        epoch = self.epoch,
                        share_index = %share.index,
                        seat = ?self.me,
                        "beacon share index does not match this node's consensus participant \
                         index; no seed partial can be produced for the epoch (logged once — \
                         the condition is frozen for the epoch and this runs per vote)"
                    );
                }
                return None;
            }
            Some(beacon::sign_seed_partial(share, &self.namespace, round).value)
        })?
    }

    fn verify_partial(&self, round: Round, index: Participant, value: &BlsSignature) -> bool {
        self.with_material(|sharing, _| {
            beacon::verify_seed_partial(
                sharing,
                &self.namespace,
                round,
                &PartialSignature::<MinSig> {
                    index,
                    value: *value,
                },
            )
        })
        .unwrap_or(false)
    }

    fn recover(
        &self,
        partials: &[(Participant, BlsSignature)],
        threshold: u32,
    ) -> Option<BlsSignature> {
        let partials: Vec<PartialSignature<MinSig>> = partials
            .iter()
            .map(|&(index, value)| PartialSignature::<MinSig> { index, value })
            .collect();
        self.with_material(|sharing, _| {
            match beacon::recover_seed_with_threshold(sharing, &partials, threshold) {
                Ok(sig) => Some(sig),
                Err(e) => {
                    // THE LOG `recover_seed_with_threshold` CANNOT WRITE ITSELF —
                    // see its docs. A threshold mismatch is frozen for the epoch
                    // (the fault model, or `committee.len()` vs the sharing's
                    // total) while `assemble` re-runs on every attestation past
                    // quorum, so unlatched this is ~n identical lines per subject
                    // per view. Latched here rather than in a `static`, because
                    // this oracle is per-epoch and the next epoch's occurrence is
                    // genuinely new.
                    if !self.warned_threshold_mismatch.swap(true, Ordering::Relaxed) {
                        error!(
                            epoch = self.epoch,
                            caller_threshold = threshold,
                            sharing_required = sharing.required::<N3f1>(),
                            sharing_total = sharing.total(),
                            ?e,
                            "seed recovery REFUSED: the vote quorum and the seed \
                             threshold disagree — either the fault model is no \
                             longer N3f1 on both halves, or this epoch's DKG was \
                             not dealt over its consensus committee. NO \
                             certificate of this epoch can be assembled until it \
                             is resolved (logged once per epoch — the condition \
                             is frozen and this runs per attestation)"
                        );
                    }
                    None
                }
            }
        })?
    }

    fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck {
        // Keyed at the epoch the CHAIN says minted the key in force here, out of
        // the artifact that mint's quorum certified — see [`KeyIndex`]. It used to
        // read a key store at the LIVE epoch, which answered only because W1
        // published this node's own reconstruction under every epoch it entered.
        //
        // SYNCHRONOUS only. "Not resolvable" IS this method's answer, not a reason
        // to go resolve: resolving would put an await on the vote path, and the
        // caller already knows what to do with `NoKey`.
        //
        // TWO OF THE THREE ARMS ARE COUNTED, and `Invalid` deliberately is not.
        // A wrong seed under a known key already has loud, attributable handling
        // at every call site (the inlet's `BLS verify FAILED` + data-fault
        // rotation, the scheme's `false`); what had no witness at all was the
        // keyed/keyless SPLIT, which is what these two make readable.
        match self.keys.key_at(self.epoch) {
            Some(pk) if beacon::verify_seed(&pk, &self.namespace, round, seed) => {
                self.metrics.seed_verify_ok.inc();
                SeedCheck::Valid
            }
            Some(_) => SeedCheck::Invalid,
            None => {
                self.metrics.seed_verify_no_key.inc();
                SeedCheck::NoKey
            }
        }
    }
}

impl fmt::Debug for BeaconOracle {
    /// Names the epoch and nothing else: the `Debug` bound on [`SeedOracle`]
    /// exists so a scheme holding one stays printable, not so key material can be
    /// logged.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BeaconOracle")
            .field("epoch", &self.epoch)
            .field("me", &self.me)
            .finish_non_exhaustive()
    }
}

/// A [`SeedOracle`] for a node that holds no threshold material and never will:
/// a `--cert-follow` follower. It obtains `PK_epoch` from its upstream's
/// artifact and can check an assembled σ against it — nothing else.
///
/// The three material-bound answers are permanently negative BY TYPE rather than
/// by state, which is the same distinction [`super::follower`] draws everywhere
/// else: a follower runs no ceremony, so "not yet" would be a lie.
#[derive(Clone)]
pub(crate) struct KeyOnlyOracle {
    pub(crate) epoch: u64,
    pub(crate) keys: KeyIndex,
    pub(crate) namespace: Vec<u8>,
    /// The same two families [`BeaconOracle`] bumps, for the same reason. A
    /// follower is where the keyless window is most ordinary — it can obtain
    /// `PK_epoch` only by fetching the epoch's artifact — so leaving this node
    /// class uncounted would leave the split invisible exactly where it is
    /// routine.
    pub(crate) metrics: BeaconMetrics,
}

impl SeedOracle for KeyOnlyOracle {
    fn sign_partial(&self, _round: Round) -> Option<BlsSignature> {
        None
    }

    fn verify_partial(&self, _round: Round, _index: Participant, _value: &BlsSignature) -> bool {
        false
    }

    fn recover(
        &self,
        _partials: &[(Participant, BlsSignature)],
        _threshold: u32,
    ) -> Option<BlsSignature> {
        None
    }

    fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck {
        match self.keys.key_at(self.epoch) {
            Some(pk) if beacon::verify_seed(&pk, &self.namespace, round, seed) => {
                self.metrics.seed_verify_ok.inc();
                SeedCheck::Valid
            }
            Some(_) => SeedCheck::Invalid,
            None => {
                self.metrics.seed_verify_no_key.inc();
                SeedCheck::NoKey
            }
        }
    }
}

impl fmt::Debug for KeyOnlyOracle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyOnlyOracle")
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::{
        actor::DETERMINISTIC_BOOTSTRAP_EPOCH,
        artifact::{key_index_over, ArtifactStore, MintFixture},
        ceremony::CeremonyOutput,
    };
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::{
        bls12381::{dkg::deal, primitives::sharing::Mode},
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, Faults as _, N3f1};
    use fluentbase_bls::{beacon::seed_namespace, fluent_namespace, PeerPubkey};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        RwLock,
    };

    const EPOCH: u64 = 9;
    /// The last epoch whose committee CHANGED, six epochs below the live one:
    /// every epoch in between carried, so nothing is keyed at [`EPOCH`].
    const CHANGE: u64 = 5;
    const N: usize = 4;

    fn round(view: u64) -> Round {
        Round::new(Epoch::new(EPOCH), View::new(view))
    }

    /// A real committee dealing to itself, as the `DkgActor` memoizes it.
    fn ceremony(seed: u64) -> (CeremonyOutput, Vec<Share>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..N).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players.clone())
                .expect("deal");
        let held = players
            .iter()
            .map(|p| shares.get_value(p).expect("share").clone())
            .collect();
        (outcome, held)
    }

    fn store(minted_at: u64, share: &Share) -> CeremonyStore {
        Arc::new(RwLock::new(std::collections::BTreeMap::from([(
            minted_at,
            share.clone(),
        )])))
    }

    /// A [`KeyIndex`] whose chain record names exactly `bits` as change epochs and
    /// whose artifact store carries `outcome` at each of them — the state of a node
    /// that can answer for the epochs those bits carry to.
    fn keys_at(outcome: &CeremonyOutput, bits: &[u64]) -> KeyIndex {
        let mints = MintFixture::new();
        for bit in bits {
            mints.mint(*bit, outcome.clone());
        }
        mints.keys.clone()
    }

    /// The same chain record with NO artifact anywhere: the keyless window.
    fn keyless_at(bits: &[u64]) -> KeyIndex {
        key_index_over(ArtifactStore::new(), bits)
    }

    fn oracle(ceremony: CeremonyStore, keys: KeyIndex, me: Option<Participant>) -> BeaconOracle {
        BeaconOracle {
            epoch: EPOCH,
            ceremony,
            keys,
            namespace: seed_namespace(&fluent_namespace(20994)),
            me,
            warned_threshold_mismatch: Arc::new(AtomicBool::new(false)),
            warned_seat_mismatch: Arc::new(AtomicBool::new(false)),
            metrics: BeaconMetrics::default(),
        }
    }

    #[test]
    fn a_quorum_of_partials_recovers_a_seed_every_member_verifies() {
        let (outcome, shares) = ceremony(0xA1);
        let keys = keys_at(&outcome, &[EPOCH]);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| oracle(store(EPOCH, s), keys.clone(), Some(s.index)))
            .collect();
        let r = round(3);

        let partials: Vec<(Participant, BlsSignature)> = members
            .iter()
            .zip(&shares)
            .map(|(o, s)| (s.index, o.sign_partial(r).expect("member signs a partial")))
            .collect();
        for member in &members {
            for &(index, ref value) in &partials {
                assert!(member.verify_partial(r, index, value));
            }
        }

        let quorum = N3f1::quorum(N as u32);
        let seed = members[0]
            .recover(&partials[..quorum as usize], quorum)
            .expect("recover from exactly a quorum");
        // `verify_seed` reads the MINT's artifact, which here is `EPOCH`'s own.
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
    }

    /// The carry: a committee stable since `CHANGE` holds its material keyed at
    /// `CHANGE` and nothing at `EPOCH`, and must sign and judge `EPOCH` normally.
    /// An exact-epoch lookup answers nothing here, and a network of such nodes
    /// stops.
    #[test]
    fn a_stable_committee_serves_the_mint_keyed_at_its_last_change_epoch() {
        let (outcome, shares) = ceremony(0xA2);
        let keys = keys_at(&outcome, &[CHANGE]);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| oracle(store(CHANGE, s), keys.clone(), Some(s.index)))
            .collect();
        let r = round(7);

        let quorum = N3f1::quorum(N as u32);
        let partials: Vec<(Participant, BlsSignature)> = members
            .iter()
            .zip(&shares)
            .take(quorum as usize)
            .map(|(o, s)| (s.index, o.sign_partial(r).expect("a carried mint signs")))
            .collect();
        for &(index, ref value) in &partials {
            assert!(members[0].verify_partial(r, index, value));
        }
        let seed = members[0]
            .recover(&partials, quorum)
            .expect("a carried mint recovers");

        // AND IT RESOLVES WITH NOTHING FILED UNDER THE LIVE EPOCH. This used to need
        // a W1 publication under `EPOCH` to pass; now the chain's `changed` record
        // names `CHANGE` and that mint's artifact answers. Reds if the lookup goes
        // back to the live epoch — which is a whole network stopping at its first
        // carry epoch, not a degradation.
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
    }

    /// THE DIVERGENCE CLASS IS UNREACHABLE, and this is what replaced the two tests
    /// that pinned its handling.
    ///
    /// They were `a_carried_mint_diverging_from_the_key_attested_at_its_mint_is_refused`
    /// and `a_fresh_mint_diverging_from_the_attested_key_judges_nothing`. Both staged a
    /// `CeremonyStore` holding one polynomial and a `BeaconKeys` attesting a DIFFERENT
    /// one for the same minting epoch, and asserted the gate refused to sign, verify
    /// or recover with it. That state cannot be staged any more: the polynomial and
    /// the attested key are the SAME field of the SAME artifact (`KeyIndex`), so the
    /// gate, its `dpos_seed_material_refused_divergent_total` counter and the soak-v39
    /// class are deleted rather than left unreachable.
    ///
    /// What is assertable, and what this asserts, is the pairing the gate existed to
    /// protect: material is served only where the share and the artifact agree on the
    /// MINTING epoch, and a share filed at the wrong epoch serves nothing — which is
    /// the one way the two halves can still fail to line up.
    #[test]
    fn material_is_served_only_where_the_share_and_the_artifact_share_a_mint() {
        let (outcome, shares) = ceremony(0xA3);
        let r = round(2);
        let keys = keys_at(&outcome, &[CHANGE]);

        // The share filed at the mint the chain names: served.
        let matched = oracle(
            store(CHANGE, &shares[0]),
            keys.clone(),
            Some(shares[0].index),
        );
        let partial = matched
            .sign_partial(r)
            .expect("a share at the chain's mint epoch is served");
        assert!(matched.verify_partial(r, shares[0].index, &partial));

        // The SAME share filed at a different epoch: the artifact's mint is `CHANGE`,
        // the share is not there, so nothing is served. No gate, no metric — the
        // lookup simply misses.
        let misfiled = oracle(
            store(EPOCH, &shares[0]),
            keys.clone(),
            Some(shares[0].index),
        );
        assert!(misfiled.sign_partial(r).is_none());
        assert!(!misfiled.verify_partial(r, shares[0].index, &partial));
        assert!(misfiled
            .recover(&[(shares[0].index, partial)], N3f1::quorum(N as u32))
            .is_none());

        // And with no artifact at all the same store serves nothing either: the
        // polynomial has one owner, so its absence is the whole refusal.
        let keyless = oracle(
            store(CHANGE, &shares[0]),
            keyless_at(&[CHANGE]),
            Some(shares[0].index),
        );
        assert!(keyless.sign_partial(r).is_none());
    }

    /// The relocated share-index binding: the share's index IS the consensus
    /// participant index (both commonware-sorted), and a partial signed under a
    /// mismatched one recovers to nothing.
    #[test]
    fn a_share_that_is_not_this_nodes_participant_index_signs_nothing() {
        let (outcome, shares) = ceremony(0xA5);
        let held = store(EPOCH, &shares[0]);

        assert!(oracle(
            held.clone(),
            keys_at(&outcome, &[EPOCH]),
            Some(shares[1].index)
        )
        .sign_partial(round(1))
        .is_none());
        assert!(oracle(held, keys_at(&outcome, &[EPOCH]), None)
            .sign_partial(round(1))
            .is_none());
    }

    #[test]
    fn a_partial_is_bound_to_its_signer_index_and_its_round() {
        let (outcome, shares) = ceremony(0xA6);
        let judge = oracle(store(EPOCH, &shares[0]), keys_at(&outcome, &[EPOCH]), None);
        let signer = oracle(
            store(EPOCH, &shares[0]),
            keys_at(&outcome, &[EPOCH]),
            Some(shares[0].index),
        );
        let r = round(5);
        let partial = signer.sign_partial(r).expect("signs");

        assert!(judge.verify_partial(r, shares[0].index, &partial));
        assert!(!judge.verify_partial(r, shares[1].index, &partial));
        assert!(!judge.verify_partial(round(6), shares[0].index, &partial));
    }

    /// The chain names a mint this node never attended: no usable material, and
    /// the carry walk must not reach past it to an older stored one.
    #[test]
    fn a_mint_this_node_never_attended_signs_nothing_and_judges_nothing() {
        let (outcome, shares) = ceremony(0xA7);
        let r = round(2);
        let partial = oracle(
            store(CHANGE, &shares[0]),
            keys_at(&outcome, &[CHANGE]),
            Some(shares[0].index),
        )
        .sign_partial(r)
        .expect("the fixture signs while the chain's mint is the stored one");

        // The chain re-minted at EPOCH; this node holds only the CHANGE mint.
        let blind = oracle(
            store(CHANGE, &shares[0]),
            keys_at(&outcome, &[CHANGE, EPOCH]),
            Some(shares[0].index),
        );
        assert!(blind.sign_partial(r).is_none());
        assert!(!blind.verify_partial(r, shares[0].index, &partial));
        assert!(blind
            .recover(&[(shares[0].index, partial)], N3f1::quorum(N as u32))
            .is_none());
    }

    /// An undecided `dkgQual` bit — the boundary window, where the epoch's
    /// committee is not committed at this node's finalized hash — must be a retry,
    /// never a verdict this oracle caches. Caching it wedges the epoch for the
    /// life of the process.
    #[test]
    fn an_undecided_dkg_qual_bit_is_retried_on_the_next_call() {
        let (outcome, shares) = ceremony(0xA8);
        // Undecided until the epoch's committee is committed at this node's
        // finalized hash, frozen at CHANGE from then on.
        let committed = Arc::new(AtomicBool::new(false));
        let seen = committed.clone();
        let changed: crate::beacon::artifact::ChangedAt =
            Arc::new(move |e| seen.load(Ordering::SeqCst).then_some(e == CHANGE));
        let artifacts = ArtifactStore::new();
        artifacts.insert(
            CHANGE,
            crate::beacon::artifact::artifact_with_key(CHANGE, outcome.clone()),
        );
        let node = oracle(
            store(CHANGE, &shares[0]),
            KeyIndex::new(artifacts, crate::beacon::artifact::MintIndex::new(changed)),
            Some(shares[0].index),
        );
        let r = round(1);

        assert!(node.sign_partial(r).is_none());
        committed.store(true, Ordering::SeqCst);
        assert!(node.sign_partial(r).is_some());
    }

    /// `verify_seed` answers `NoKey` until the MINT's artifact is held, and the lookup
    /// is at the mint rather than at the live epoch.
    ///
    /// It used to say "until the key STORE holds the epoch key", and what put it there
    /// was W1 — a per-entered-epoch publication of this node's own reconstruction.
    /// With W1 gone (П-3) the source is the artifact of the epoch the chain's `changed`
    /// record names, and `EPOCH` here is a CARRY epoch whose mint is `CHANGE`.
    #[test]
    fn verify_seed_answers_no_key_until_the_mints_artifact_is_held() {
        let (outcome, shares) = ceremony(0xA9);
        // The share is held but the artifact is NOT: signing needs the polynomial, so
        // the fixture mints into a store it keeps a handle on and lands it later.
        let mints = MintFixture::new();
        mints.mint(CHANGE, outcome.clone());
        let keys = mints.keys.clone();
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| oracle(store(CHANGE, s), keys.clone(), Some(s.index)))
            .collect();
        let r = round(4);
        let quorum = N3f1::quorum(N as u32);
        let partials: Vec<(Participant, BlsSignature)> = members
            .iter()
            .zip(&shares)
            .take(quorum as usize)
            .map(|(o, s)| (s.index, o.sign_partial(r).expect("signs")))
            .collect();
        let seed = members[0].recover(&partials, quorum).expect("recover");

        // The mint IS held here, so the key resolves at the CARRY epoch with nothing
        // filed under it — which is the property W1's removal turns on.
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
        assert_eq!(members[0].verify_seed(round(5), &seed), SeedCheck::Invalid);

        // And the keyless window, with the same chain record and no artifact: `NoKey`,
        // never `Invalid` — a node that cannot resolve the key accuses nobody.
        let keyless = oracle(
            store(CHANGE, &shares[0]),
            keyless_at(&[CHANGE]),
            Some(shares[0].index),
        );
        assert_eq!(keyless.verify_seed(r, &seed), SeedCheck::NoKey);
    }

    /// THE POSITIVE EDGE THAT REPLACED A DELETED LOG LINE, pinned in both
    /// directions.
    ///
    /// `smoke-vrf-dkg-live-heal` used to witness "this epoch left vote-only
    /// admission" by grepping `epoch scheme upgraded to PINNED` out of
    /// `EpochSchemeProvider::register`. FLU-1202 deleted the pin, so that line has
    /// no emitter and the case's leg had no witness at all. `seed_verify_ok` is
    /// the replacement, and it is only a replacement if it moves EXACTLY on the
    /// keyed transition — so this asserts the whole shape: flat while keyless,
    /// climbing once the key lands, and never moved by a seed that failed.
    #[test]
    fn the_seed_verify_counters_split_the_keyless_window_from_the_keyed_one() {
        let (outcome, shares) = ceremony(0xAB);
        let keys = keys_at(&outcome, &[CHANGE]);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| oracle(store(CHANGE, s), keys.clone(), Some(s.index)))
            .collect();
        let r = round(4);
        let quorum = N3f1::quorum(N as u32);
        let partials: Vec<(Participant, BlsSignature)> = members
            .iter()
            .zip(&shares)
            .take(quorum as usize)
            .map(|(o, s)| (s.index, o.sign_partial(r).expect("signs")))
            .collect();
        let seed = members[0].recover(&partials, quorum).expect("recover");

        // The KEYLESS window, on a node with the same chain record and no artifact.
        let keyless = oracle(
            store(CHANGE, &shares[0]),
            keyless_at(&[CHANGE]),
            Some(shares[0].index),
        );
        let k = &keyless.metrics;
        assert_eq!(keyless.verify_seed(r, &seed), SeedCheck::NoKey);
        assert_eq!(keyless.verify_seed(r, &seed), SeedCheck::NoKey);
        assert_eq!(
            (k.seed_verify_no_key.get(), k.seed_verify_ok.get()),
            (2, 0),
            "every certificate of the keyless window is counted, not just the first"
        );

        // The KEYED one, on the node that holds the mint's artifact.
        let m = &members[0].metrics;
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
        assert_eq!(
            (m.seed_verify_no_key.get(), m.seed_verify_ok.get()),
            (0, 1),
            "the ok counter's 0 -> non-zero edge IS the witness the smoke leg reads"
        );

        // `Invalid` moves NEITHER. A wrong seed under a known key already has loud
        // attributable handling at every call site; folding it into either family
        // would make the keyed/keyless split unreadable, which is the only thing
        // these two exist to say.
        assert_eq!(members[0].verify_seed(round(5), &seed), SeedCheck::Invalid);
        assert_eq!((m.seed_verify_no_key.get(), m.seed_verify_ok.get()), (0, 1));
    }

    /// The bootstrap epoch mints unconditionally, so an all-clear bit history
    /// bottoms out there rather than answering "no mint".
    #[test]
    fn an_all_clear_bit_history_serves_the_bootstrap_mint() {
        let (outcome, shares) = ceremony(0xAA);
        // The bootstrap epoch's own artifact, and NO bit set anywhere above it: the
        // walk bottoms out at the bootstrap mint rather than answering "no mint".
        let node = oracle(
            store(DETERMINISTIC_BOOTSTRAP_EPOCH, &shares[0]),
            keys_at(&outcome, &[DETERMINISTIC_BOOTSTRAP_EPOCH]),
            Some(shares[0].index),
        );
        assert!(node.sign_partial(round(1)).is_some());
    }
}
