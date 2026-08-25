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
//! to `share_probe`, which runs per reconcile and owns `dpos_carry_forward_refused_total`
//! — the same refusal at per-vote cadence would inflate that counter by two orders
//! of magnitude and destroy what it means.
//!
//! The families this type owns are therefore its OWN, not the resolver's, and
//! their per-vote cadence is part of their contract rather than a defect in it:
//! `dpos_seed_verify_{ok,no_key}_total` and
//! `dpos_seed_material_refused_divergent_total` are read as edges and rates, and
//! nothing compares them against a per-reconcile family. Bumping the resolver's
//! counter from here instead is the specific mistake this split exists to
//! prevent.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
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

use super::{
    actor::CeremonyStore,
    carry::{select_carry_scheme, CarryVerdict, DkgQualFor},
    ceremony::CeremonyOutput,
    keys::BeaconKeys,
    metrics::BeaconMetrics,
    resolve::mint_diverges_from_attested,
};

/// The beacon-backed [`SeedOracle`] for ONE epoch.
///
/// Reads only the two synchronous rungs — the ceremony store and the group-key
/// store's raw probe. The asynchronous key ladder (`BeaconKeys::get_pk`) is
/// deliberately not reachable from here: a miss on the sync rung is the
/// [`SeedCheck::NoKey`] answer, and acquisition happens out of band.
#[derive(Clone)]
pub(crate) struct BeaconOracle {
    /// The epoch this oracle answers for — the TARGET of the carry arbitration,
    /// not necessarily the epoch its material was minted at. Its callers check
    /// the round's epoch binding before delegating, so nothing here re-derives it
    /// from a `Round`.
    pub(crate) epoch: u64,
    pub(crate) ceremony: CeremonyStore,
    pub(crate) keys: BeaconKeys,
    pub(crate) dkg_qual: DkgQualFor,
    /// The chain's beacon seed-signing namespace.
    pub(crate) namespace: Vec<u8>,
    /// This node's consensus participant index for the epoch, `None` for a
    /// verifier-flavored oracle. The share's own index must equal it or the
    /// partial is mis-attributed and recovers to nothing.
    pub(crate) me: Option<Participant>,
    /// Memo of [`Self::epoch`]'s mint epoch. Construct it holding `None`.
    pub(crate) minted_at: Arc<Mutex<Option<u64>>>,
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
    /// The chain's key epoch for [`Self::epoch`], memoised.
    ///
    /// Only a `Serve` verdict is recorded, and recording it is safe because the
    /// epoch→mint mapping is a frozen chain fact: `dkgQual` bits are immutable
    /// once their epoch's committee is committed. `NoUsableMint` and `ReadFailed`
    /// are NOT recorded — an epoch whose committee is not committed at this node's
    /// finalized hash reads as undecided, and caching that would wedge the epoch
    /// for the life of the process instead of retrying on the next vote.
    ///
    /// The memo covers the chain walk ONLY. Whether the store still holds the
    /// mint is re-read every call, so retention pruning the entry is answered as
    /// a miss rather than served from a stale verdict.
    fn minted_at_for(&self, held: &BTreeMap<u64, (CeremonyOutput, Share)>) -> Option<u64> {
        if let Some(hit) = self.minted_at.lock().ok().and_then(|memo| *memo) {
            return Some(hit);
        }
        // `select_carry_scheme` consults `DkgQualFor`, and `frozen_dkg_qual` does
        // not cache an undecided bit — so this call can reach chain state. It is
        // behind the memo for that reason: once per epoch is a probe, once per
        // attestation would be a state read per vote through the boundary window.
        let CarryVerdict::Serve { minted_at } =
            select_carry_scheme(self.epoch, |e| held.contains_key(&e), &self.dkg_qual)
        else {
            return None;
        };
        if let Ok(mut memo) = self.minted_at.lock() {
            *memo = Some(minted_at);
        }
        Some(minted_at)
    }

    /// The polynomial and share this node may judge [`Self::epoch`] with, under
    /// ONE ceremony-store read and with no clone: the closure borrows both.
    ///
    /// A stable committee writes no new `CeremonyStore` entry, so the material for
    /// `epoch` is keyed at the last CHANGE epoch and an exact `get(&epoch)` misses
    /// on every carried epoch — the carry walk IS the lookup, not an enrichment of
    /// it.
    ///
    /// THE DIVERGENCE GATE runs here, on every call, against the key attested at
    /// the MINT epoch. `share_probe` withholds a node whose polynomial diverges but
    /// does NOT correct the ceremony store, so the divergent entry stays; before
    /// this oracle existed that was harmless, because a withheld node was handed a
    /// scheme with no beacon material and its polynomial judged nothing. The
    /// verdict is deliberately NOT memoised beside `minted_at`: `attested` moves
    /// `None → Agreed` mid-epoch and reveals a mint that was being served until
    /// then, which is the soak-v39 poisoning exactly.
    ///
    /// A poisoned lock degrades to a miss rather than propagating a panic onto the
    /// vote path.
    fn with_material<T>(&self, f: impl FnOnce(&Sharing<MinSig>, &Share) -> T) -> Option<T> {
        let held = self.ceremony.read().ok()?;
        let minted_at = self.minted_at_for(&held)?;
        let (outcome, share) = held.get(&minted_at)?;
        let sharing = outcome.public();
        if mint_diverges_from_attested(&self.keys, minted_at, sharing.public()) {
            // COUNTED, and on this family rather than the resolver's: see the
            // module docs. Until this counter existed the refusal was entirely
            // invisible — a node whose local polynomial diverged simply answered
            // "no partial" and "cannot verify", which reads exactly like a node
            // that never had material at all.
            self.metrics.seed_material_refused_divergent.inc();
            return None;
        }
        Some(f(sharing, share))
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
        // Keyed at the LIVE epoch, unlike the material: W1 publishes `PK_epoch`
        // into this store under the epoch it is in force for, whatever minted it.
        //
        // The SYNCHRONOUS key rung only. "Not cached" IS this method's answer, not
        // a reason to go resolve: resolving would put an await on the vote path,
        // and the caller already knows what to do with `NoKey`.
        //
        // TWO OF THE THREE ARMS ARE COUNTED, and `Invalid` deliberately is not.
        // A wrong seed under a known key already has loud, attributable handling
        // at every call site (the inlet's `BLS verify FAILED` + data-fault
        // rotation, the scheme's `false`); what had no witness at all was the
        // keyed/keyless SPLIT, which is what these two make readable.
        match self.keys.cached_only(self.epoch) {
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
    pub(crate) keys: BeaconKeys,
    pub(crate) namespace: Vec<u8>,
    /// The same two families [`BeaconOracle`] bumps, for the same reason. A
    /// follower is where the keyless window is most ordinary — it can obtain
    /// `PK_epoch` only by fetching the epoch's artifact — so leaving this node
    /// class uncounted would leave the split invisible exactly where it is
    /// routine. There is no divergence arm: a follower holds no local mint to
    /// diverge.
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
        match self.keys.cached_only(self.epoch) {
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
    use crate::beacon::{actor::DETERMINISTIC_BOOTSTRAP_EPOCH, keys::KeySource};
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
    use std::{
        collections::BTreeSet,
        sync::{
            atomic::{AtomicBool, Ordering},
            RwLock,
        },
    };

    const EPOCH: u64 = 9;
    /// The last epoch whose committee CHANGED, six epochs below the live one:
    /// every epoch in between carried, so nothing is keyed at [`EPOCH`].
    const CHANGE: u64 = 5;
    const N: usize = 4;

    fn round(view: u64) -> Round {
        Round::new(Epoch::new(EPOCH), View::new(view))
    }

    /// A frozen on-chain `dkgQual` history: exactly these epochs re-minted.
    fn qual(bits: &[u64]) -> DkgQualFor {
        let set: BTreeSet<u64> = bits.iter().copied().collect();
        Arc::new(move |e| Some(set.contains(&e)))
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

    fn store(minted_at: u64, outcome: &CeremonyOutput, share: &Share) -> CeremonyStore {
        Arc::new(RwLock::new(BTreeMap::from([(
            minted_at,
            (outcome.clone(), share.clone()),
        )])))
    }

    fn oracle(
        ceremony: CeremonyStore,
        keys: BeaconKeys,
        dkg_qual: DkgQualFor,
        me: Option<Participant>,
    ) -> BeaconOracle {
        BeaconOracle {
            epoch: EPOCH,
            ceremony,
            keys,
            dkg_qual,
            namespace: seed_namespace(&fluent_namespace(20994)),
            me,
            minted_at: Arc::new(Mutex::new(None)),
            warned_threshold_mismatch: Arc::new(AtomicBool::new(false)),
            warned_seat_mismatch: Arc::new(AtomicBool::new(false)),
            metrics: BeaconMetrics::default(),
        }
    }

    /// A group-key store holding `outcome`'s key as attested AT `minted_at` — the
    /// state once that mint's agreement wrote back.
    fn attested_at(minted_at: u64, outcome: &CeremonyOutput) -> BeaconKeys {
        let keys = BeaconKeys::new();
        keys.set_pk(minted_at, *outcome.public().public(), KeySource::Agreed);
        keys
    }

    #[test]
    fn a_quorum_of_partials_recovers_a_seed_every_member_verifies() {
        let (outcome, shares) = ceremony(0xA1);
        let keys = attested_at(EPOCH, &outcome);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| {
                oracle(
                    store(EPOCH, &outcome, s),
                    keys.clone(),
                    qual(&[EPOCH]),
                    Some(s.index),
                )
            })
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
        // `verify_seed` reads the LIVE epoch's entry, which here is also the mint.
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
    }

    /// The carry: a committee stable since `CHANGE` holds its material keyed at
    /// `CHANGE` and nothing at `EPOCH`, and must sign and judge `EPOCH` normally.
    /// An exact-epoch lookup answers nothing here, and a network of such nodes
    /// stops.
    #[test]
    fn a_stable_committee_serves_the_mint_keyed_at_its_last_change_epoch() {
        let (outcome, shares) = ceremony(0xA2);
        let keys = attested_at(CHANGE, &outcome);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| {
                oracle(
                    store(CHANGE, &outcome, s),
                    keys.clone(),
                    qual(&[CHANGE]),
                    Some(s.index),
                )
            })
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

        // W1 publishes PK_epoch under the LIVE epoch even when the mint is older.
        keys.set_pk(EPOCH, *outcome.public().public(), KeySource::LocalDkg);
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
    }

    /// A carried mint is gated against the key attested AT ITS MINT EPOCH.
    /// Comparing at the target epoch instead is vacuous on exactly the epochs the
    /// carry serves — `attested` answers only for the `Agreed` tier, and a carry
    /// epoch runs no agreement — so this asserts BOTH halves: the wrong comparand
    /// serves the divergent mint, the right one refuses it.
    #[test]
    fn a_carried_mint_diverging_from_the_key_attested_at_its_mint_is_refused() {
        let (mine, shares) = ceremony(0xA3);
        let (theirs, _) = ceremony(0xB3);
        let held = store(CHANGE, &mine, &shares[0]);
        let r = round(2);

        let ungated = oracle(
            held.clone(),
            BeaconKeys::new(),
            qual(&[CHANGE]),
            Some(shares[0].index),
        );
        let partial = ungated
            .sign_partial(r)
            .expect("the same material must serve while nothing is attested");
        assert!(ungated.verify_partial(r, shares[0].index, &partial));

        let attested_at_target = oracle(
            held.clone(),
            attested_at(EPOCH, &theirs),
            qual(&[CHANGE]),
            Some(shares[0].index),
        );
        assert!(
            attested_at_target.sign_partial(r).is_some(),
            "an attestation at the TARGET epoch cannot gate a carried mint — if this \
             starts refusing, the comparand moved and the test below proves nothing"
        );

        let diverged = oracle(
            held,
            attested_at(CHANGE, &theirs),
            qual(&[CHANGE]),
            Some(shares[0].index),
        );
        assert!(diverged.sign_partial(r).is_none());
        assert!(!diverged.verify_partial(r, shares[0].index, &partial));
        assert!(diverged
            .recover(&[(shares[0].index, partial)], N3f1::quorum(N as u32))
            .is_none());
    }

    /// A fresh mint at the live epoch is gated the same way — `minted_at == epoch`
    /// is just the degenerate carry.
    #[test]
    fn a_fresh_mint_diverging_from_the_attested_key_judges_nothing() {
        let (mine, shares) = ceremony(0xA4);
        let (theirs, _) = ceremony(0xB4);
        let held = store(EPOCH, &mine, &shares[0]);
        let r = round(1);

        let ungated = oracle(
            held.clone(),
            BeaconKeys::new(),
            qual(&[EPOCH]),
            Some(shares[0].index),
        );
        let partial = ungated
            .sign_partial(r)
            .expect("signs while nothing attested");

        let diverged = oracle(
            held,
            attested_at(EPOCH, &theirs),
            qual(&[EPOCH]),
            Some(shares[0].index),
        );
        assert!(diverged.sign_partial(r).is_none());
        assert!(!diverged.verify_partial(r, shares[0].index, &partial));
    }

    /// The relocated share-index binding: the share's index IS the consensus
    /// participant index (both commonware-sorted), and a partial signed under a
    /// mismatched one recovers to nothing.
    #[test]
    fn a_share_that_is_not_this_nodes_participant_index_signs_nothing() {
        let (outcome, shares) = ceremony(0xA5);
        let held = store(EPOCH, &outcome, &shares[0]);

        assert!(oracle(
            held.clone(),
            attested_at(EPOCH, &outcome),
            qual(&[EPOCH]),
            Some(shares[1].index)
        )
        .sign_partial(round(1))
        .is_none());
        assert!(
            oracle(held, attested_at(EPOCH, &outcome), qual(&[EPOCH]), None)
                .sign_partial(round(1))
                .is_none()
        );
    }

    #[test]
    fn a_partial_is_bound_to_its_signer_index_and_its_round() {
        let (outcome, shares) = ceremony(0xA6);
        let judge = oracle(
            store(EPOCH, &outcome, &shares[0]),
            attested_at(EPOCH, &outcome),
            qual(&[EPOCH]),
            None,
        );
        let signer = oracle(
            store(EPOCH, &outcome, &shares[0]),
            attested_at(EPOCH, &outcome),
            qual(&[EPOCH]),
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
            store(CHANGE, &outcome, &shares[0]),
            attested_at(CHANGE, &outcome),
            qual(&[CHANGE]),
            Some(shares[0].index),
        )
        .sign_partial(r)
        .expect("the fixture signs while the chain's mint is the stored one");

        // The chain re-minted at EPOCH; this node holds only the CHANGE mint.
        let blind = oracle(
            store(CHANGE, &outcome, &shares[0]),
            attested_at(CHANGE, &outcome),
            qual(&[CHANGE, EPOCH]),
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
        let dkg_qual: DkgQualFor =
            Arc::new(move |e| seen.load(Ordering::SeqCst).then_some(e == CHANGE));
        let node = oracle(
            store(CHANGE, &outcome, &shares[0]),
            attested_at(CHANGE, &outcome),
            dkg_qual,
            Some(shares[0].index),
        );
        let r = round(1);

        assert!(node.sign_partial(r).is_none());
        committed.store(true, Ordering::SeqCst);
        assert!(node.sign_partial(r).is_some());
    }

    #[test]
    fn verify_seed_answers_no_key_until_the_key_store_holds_the_epoch_key() {
        let (outcome, shares) = ceremony(0xA9);
        let keys = attested_at(CHANGE, &outcome);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| {
                oracle(
                    store(CHANGE, &outcome, s),
                    keys.clone(),
                    qual(&[CHANGE]),
                    Some(s.index),
                )
            })
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

        // The mint's key is attested, but nothing is recorded under the LIVE epoch
        // yet — the window a certificate is admitted on its multisig half alone.
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::NoKey);

        keys.set_pk(EPOCH, *outcome.public().public(), KeySource::LocalDkg);
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
        assert_eq!(members[0].verify_seed(round(5), &seed), SeedCheck::Invalid);
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
        let keys = attested_at(CHANGE, &outcome);
        let members: Vec<BeaconOracle> = shares
            .iter()
            .map(|s| {
                oracle(
                    store(CHANGE, &outcome, s),
                    keys.clone(),
                    qual(&[CHANGE]),
                    Some(s.index),
                )
            })
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
        let m = &members[0].metrics;

        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::NoKey);
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::NoKey);
        assert_eq!(
            (m.seed_verify_no_key.get(), m.seed_verify_ok.get()),
            (2, 0),
            "every certificate of the keyless window is counted, not just the first"
        );

        keys.set_pk(EPOCH, *outcome.public().public(), KeySource::LocalDkg);
        assert_eq!(members[0].verify_seed(r, &seed), SeedCheck::Valid);
        assert_eq!(
            (m.seed_verify_no_key.get(), m.seed_verify_ok.get()),
            (2, 1),
            "the ok counter's 0 -> non-zero edge IS the witness the smoke leg reads"
        );

        // `Invalid` moves NEITHER. A wrong seed under a known key already has loud
        // attributable handling at every call site; folding it into either family
        // would make the keyed/keyless split unreadable, which is the only thing
        // these two exist to say.
        assert_eq!(members[0].verify_seed(round(5), &seed), SeedCheck::Invalid);
        assert_eq!((m.seed_verify_no_key.get(), m.seed_verify_ok.get()), (2, 1));
    }

    /// The refusal that had NO witness at all before this counter.
    ///
    /// A node whose local polynomial diverges from the key attested at its mint
    /// answers "no partial" and "cannot verify" — byte-identical to a node that
    /// never held material. `share_probe` withholds such a node but does not
    /// correct the ceremony store, so the divergent entry stays and the oracle
    /// keeps refusing from it, silently, for as long as the epoch lasts.
    #[test]
    fn a_divergent_mint_counts_every_refusal_it_makes() {
        let (mine, shares) = ceremony(0xAC);
        let (theirs, _) = ceremony(0xBC);
        let diverged = oracle(
            store(CHANGE, &mine, &shares[0]),
            attested_at(CHANGE, &theirs),
            qual(&[CHANGE]),
            Some(shares[0].index),
        );
        let r = round(2);
        // A genuine partial over the same material, produced while nothing is
        // attested — so the refusal below is the divergence gate and not a
        // malformed input.
        let partial = oracle(
            store(CHANGE, &mine, &shares[0]),
            BeaconKeys::new(),
            qual(&[CHANGE]),
            Some(shares[0].index),
        )
        .sign_partial(r)
        .expect("ungated material signs");
        assert_eq!(diverged.metrics.seed_material_refused_divergent.get(), 0);

        assert!(diverged.sign_partial(r).is_none());
        assert!(!diverged.verify_partial(r, shares[0].index, &partial));
        assert_eq!(
            diverged.metrics.seed_material_refused_divergent.get(),
            2,
            "counted per REFUSAL, on this type's own family — bumping the resolver's \
             per-reconcile `dpos_carry_forward_refused_total` from the vote path is the \
             specific mistake the module docs forbid"
        );
    }

    /// The bootstrap epoch mints unconditionally, so an all-clear bit history
    /// bottoms out there rather than answering "no mint".
    #[test]
    fn an_all_clear_bit_history_serves_the_bootstrap_mint() {
        let (outcome, shares) = ceremony(0xAA);
        let node = oracle(
            store(DETERMINISTIC_BOOTSTRAP_EPOCH, &outcome, &shares[0]),
            attested_at(DETERMINISTIC_BOOTSTRAP_EPOCH, &outcome),
            qual(&[]),
            Some(shares[0].index),
        );
        assert!(node.sign_partial(round(1)).is_some());
    }
}
