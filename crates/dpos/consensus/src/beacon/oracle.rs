//! The beacon's synchronous face for the consensus scheme: the one place the
//! per-epoch threshold material is read on the vote path.
//!
//! The scheme holds no key material and asks this type instead, so the share and the
//! public polynomial never leave the beacon. Every answer is derived from live
//! shared state — the ceremony store the `DkgActor` writes and the group-key store —
//! rather than a snapshot taken when the scheme was built, which lets a scheme built
//! before its epoch's ceremony finished start answering the moment the entry lands.
//!
//! This type answers; it never decides participation. The demote decision belongs to
//! `share_probe`, which runs per reconcile.

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
    /// This node's own share per minting epoch. The polynomial it pairs with comes from
    /// [`Self::keys`], keyed by the same minting epoch, so the pair cannot disagree.
    pub(crate) ceremony: CeremonyStore,
    pub(crate) keys: KeyIndex,
    /// The chain's beacon seed-signing namespace.
    pub(crate) namespace: Vec<u8>,
    /// This node's consensus participant index for the epoch, `None` for a
    /// verifier-flavored oracle. The share's own index must equal it or the
    /// partial is mis-attributed and recovers to nothing.
    pub(crate) me: Option<Participant>,
    /// First-refusal latch for the vote-quorum/seed-threshold mismatch in
    /// [`SeedOracle::recover`]. Construct it holding `false`. Separate from the seat
    /// latch because the two conditions are independent.
    pub(crate) warned_threshold_mismatch: Arc<AtomicBool>,
    /// First-refusal latch for the seat/share-index mismatch in
    /// [`SeedOracle::sign_partial`]. Construct it holding `false`. The condition is
    /// frozen for the epoch while the check runs on the per-vote path, so without the
    /// latch one misconfigured epoch warns per subject per view.
    pub(crate) warned_seat_mismatch: Arc<AtomicBool>,
    /// The beacon's counters.
    pub(crate) metrics: BeaconMetrics,
}

impl BeaconOracle {
    /// The polynomial and share this node may judge [`Self::epoch`] with.
    ///
    /// A stable committee writes no new ceremony-store entry, so the material is keyed
    /// at the last change epoch and an exact `get(&epoch)` misses on every carried
    /// epoch; [`KeyIndex`] is what performs the mint lookup.
    ///
    /// The polynomial comes from the artifact and the share from this node's own store,
    /// both keyed by the same minting epoch, so nothing that reaches here can disagree
    /// with itself. A poisoned lock degrades to a miss rather than panicking on the
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
                    // A threshold mismatch is frozen for the epoch while `assemble`
                    // re-runs on every attestation past quorum, so unlatched this is
                    // many identical lines per subject per view. Latched here rather
                    // than in a static, because this oracle is per-epoch and the next
                    // epoch's occurrence is genuinely new.
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
        // Keyed at the epoch the chain says minted the key in force, out of the artifact
        // that mint's quorum certified. Synchronous only: "not resolvable" is this method's
        // answer, not a reason to resolve, which would put an await on the vote path.
        //
        // All three arms are counted: the split between keyed and keyless is what the
        // callers read.
        match self.keys.key_at(self.epoch) {
            Some(pk) if beacon::verify_seed(&pk, &self.namespace, round, seed) => {
                self.metrics.seed_verify_ok.inc();
                SeedCheck::Valid
            }
            Some(_) => {
                self.metrics.seed_verify_invalid.inc();
                SeedCheck::Invalid
            }
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
    /// The last epoch whose committee changed, six epochs below the live one:
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

    /// The carry: a committee stable since `CHANGE` holds its material keyed at `CHANGE`
    /// and nothing at `EPOCH`, and must sign and judge `EPOCH` normally. An exact-epoch
    /// lookup answers nothing here, and a network of such nodes stops.
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

        // It resolves with nothing filed under the live epoch, because the chain's `changed`
        // record names `CHANGE` and that mint's artifact answers.
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
    }

    /// The divergence class is unreachable. The polynomial and the attested key are the
    /// same field of the same artifact, so the gate that compared them is deleted rather
    /// than left unreachable.
    ///
    /// What is assertable is the pairing the gate protected: material is served only
    /// where the share and the artifact agree on the minting epoch, and a share filed at
    /// the wrong epoch serves nothing.
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

        let keyless = oracle(
            store(CHANGE, &shares[0]),
            keyless_at(&[CHANGE]),
            Some(shares[0].index),
        );
        assert!(keyless.sign_partial(r).is_none());
    }

    /// The share's index is the consensus participant index, and a partial signed under
    /// a mismatched one recovers to nothing.
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

    /// An undecided `dkgQual` bit must be a retry, never a verdict this oracle caches;
    /// caching it would wedge the epoch for the life of the process.
    #[test]
    fn an_undecided_dkg_qual_bit_is_retried_on_the_next_call() {
        let (outcome, shares) = ceremony(0xA8);
        let committed = Arc::new(AtomicBool::new(false));
        let seen = committed.clone();
        let changed: crate::beacon::artifact::ChangedAt =
            Arc::new(move |e| seen.load(Ordering::SeqCst).then_some(e == CHANGE));
        let artifacts = ArtifactStore::new();
        assert!(artifacts
            .insert(
                CHANGE,
                crate::beacon::artifact::artifact_with_key(CHANGE, outcome.clone()),
            )
            .is_ok());
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

    /// `verify_seed` answers `NoKey` until the mint's artifact is held, and the lookup is
    /// at the mint rather than at the live epoch.
    #[test]
    fn verify_seed_answers_no_key_until_the_mints_artifact_is_held() {
        let (outcome, shares) = ceremony(0xA9);
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

        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
        assert_eq!(members[0].verify_seed(round(5), &seed), SeedCheck::Invalid);

        let keyless = oracle(
            store(CHANGE, &shares[0]),
            keyless_at(&[CHANGE]),
            Some(shares[0].index),
        );
        assert_eq!(keyless.verify_seed(r, &seed), SeedCheck::NoKey);
    }

    /// The positive edge that replaced a deleted log line. `seed_verify_ok` is the
    /// replacement witness, and it is only a replacement if it moves exactly on the
    /// keyed transition, so this asserts the whole shape: flat while keyless, climbing
    /// once the key lands, and never moved by a failed seed.
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

        let m = &members[0].metrics;
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
        assert_eq!(
            (m.seed_verify_no_key.get(), m.seed_verify_ok.get()),
            (0, 1),
            "the ok counter's 0 -> non-zero edge IS the witness the smoke leg reads"
        );

        assert_eq!(members[0].verify_seed(round(5), &seed), SeedCheck::Invalid);
        assert_eq!((m.seed_verify_no_key.get(), m.seed_verify_ok.get()), (0, 1));
    }

    /// The bootstrap epoch mints unconditionally, so an all-clear bit history bottoms
    /// out there rather than answering "no mint".
    #[test]
    fn an_all_clear_bit_history_serves_the_bootstrap_mint() {
        let (outcome, shares) = ceremony(0xAA);
        let node = oracle(
            store(DETERMINISTIC_BOOTSTRAP_EPOCH, &shares[0]),
            keys_at(&outcome, &[DETERMINISTIC_BOOTSTRAP_EPOCH]),
            Some(shares[0].index),
        );
        assert!(node.sign_partial(round(1)).is_some());
    }
}
