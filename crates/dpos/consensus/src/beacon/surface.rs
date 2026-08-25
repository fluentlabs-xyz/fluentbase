//! The consensus-facing surface of the randomness subsystem.
//!
//! The core knows two things: the seed for a round, and whether it may sign at
//! an epoch. It does not know that a DKG exists, that a key is agreed on a
//! second consensus plane, or that an artifact is a thing that can be pulled
//! from a peer. Substituting an implementation that computes the seed from a
//! hash must not require a single change above this line.

use super::{
    actor::CeremonyStore,
    carry::DkgQualFor,
    keys::{pk_prefix, BeaconKeys, InvalidSeed, KeySource},
    metrics::BeaconMetrics,
    oracle::BeaconOracle,
    resolve::BeaconVerify,
    seed::Seed,
    verified_seed::VerifiedSeed,
};
use commonware_consensus::types::{Epoch, Round, View};
use commonware_cryptography::bls12381::primitives::{
    group::Share, sharing::Sharing, variant::MinSig,
};
use commonware_runtime::Metrics;
use commonware_utils::{ordered::Error as OrderedError, Participant};
#[cfg(test)]
use fluentbase_bls::oracle::SeedCheck;
use fluentbase_bls::{
    beacon as beacon_bls, beacon::GroupPublic, keys::ValidatorBlsKeypair, oracle::SeedOracle,
    BlsSignature, Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use tokio::sync::Notify;
use tracing::{debug, info, warn};

/// Per-epoch threshold material this node holds: the public polynomial, its
/// share (`None` for a verifier-only node) and the seed namespace.
///
/// `pub(crate)` and no wider. It used to be `fluentbase_bls::scheme::BeaconKey`,
/// because the scheme was handed a copy of it; the scheme reads through the
/// oracle now, so what is left of this is the promote gates' input and it never
/// crosses the crate edge.
pub(crate) type BeaconKey = (Sharing<MinSig>, Option<Share>, Vec<u8>);

/// The witness arm's verdict. Each variant maps to a DIFFERENT vote decision, so
/// collapsing any two changes how the node votes. `NoKey` is STRUCTURAL ("this
/// node cannot know; re-polling cannot help"), `Undecided` is TRANSIENT ("ask
/// again next tick, never cache").
#[derive(Debug, PartialEq, Eq)]
pub enum WitnessCheck {
    Valid,
    Invalid,
    NoKey,
    Undecided,
}

/// Why participation was withheld. Named rather than a bool because each arm
/// owns its own metric family, and those names are scraped by the devnet smoke.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WithheldReason {
    NoUsableShare,
    KeyDivergence,
    BadShare,
}

/// The cheap half of the participation question, answered where the share-gate
/// sits today — BEFORE the boundary-block lookup, so a shareless member returns
/// without paying a marshal read.
///
/// Deliberately NOT [`SignerVerdict`]: at the probe's position there is no
/// scheme to hand back. The committee has not been checked and the keypair is
/// not one of the probe's inputs.
#[derive(Debug, PartialEq, Eq)]
pub enum ShareProbe {
    Ready,
    Withheld(WithheldReason),
}

/// May this node sign at an epoch, and with what scheme?
pub enum SignerVerdict {
    /// Sign with this scheme. ALSO the answer where [`Randomness::mandatory_at`]
    /// is false: a pre-beacon epoch has a legitimate pure-multisig signer, so
    /// this arm means "may sign", not "beacon-active".
    Signs(BlsScheme),
    /// This node's BLS key is not in the committee's map — a misconfiguration
    /// safety net, not a wedge path. Verify-only.
    RotatedKey(BlsScheme),
    /// The committee snapshot is unusable (duplicate participant keys). The
    /// caller skips the epoch, as it does today when the engine's own decode
    /// fails.
    InvalidCommittee(OrderedError),
    /// Do not sign at this epoch.
    Withheld(WithheldReason),
}

/// How much a key acquisition may spend in LATENCY. Nothing else: the provenance
/// floor is the same at both efforts, because it follows from what the key is
/// USED for rather than from how hard the caller was willing to look — see
/// [`Randomness::ensure_key`]'s implementation for the asymmetry that fixes it.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PinEffort {
    /// Store plus locally held material. No network. The variant a vote path or
    /// a per-certificate path may call.
    Local,
    /// May additionally spend one bounded peer pull. Off-path callers only.
    Thorough,
}

/// Everything the consensus core is allowed to know about randomness.
pub trait Randomness: Send + Sync {
    /// Hand over a seed that verified against its epoch key.
    ///
    /// The witness carries its own round, so there is no second argument to get
    /// wrong, and no implementation can be handed a σ nobody checked.
    ///
    /// ORDERING-CRITICAL: the caller invokes this SYNCHRONOUSLY, from inside the
    /// simplex reporter. Sync by signature so no implementation can move the
    /// record behind an await — the voter awaits that reporter before advancing
    /// the view, and the next view's leader reads this memo to embed the
    /// parent-seed witness. `spec_exec.rs` carries the derivation, including
    /// which half of the older constraint turned out not to be load-bearing.
    fn record_seed(&self, verified: VerifiedSeed);

    /// Hold a σ that arrived from the network for an epoch whose key is not
    /// resolvable here yet.
    ///
    /// Separate from [`record_seed`](Self::record_seed) by design: the two take
    /// different types because they mean different things, and no caller can
    /// reach the served map with a value it did not check.
    fn quarantine_seed(&self, round: Round, seed: BlsSignature);

    /// What a σ that failed `verify_seed` for `epoch` means, and how loudly.
    /// Delegates to [`BeaconKeys::on_invalid_seed`], which owns the rule because
    /// it owns provenance.
    fn on_invalid_seed(&self, epoch: u64) -> InvalidSeed;

    /// Ask peers for σ of `round` and wait a bounded time for it.
    ///
    /// `true` iff the served map holds it when the future resolves. The one
    /// ASYNC member of the seed half, and deliberately so: acquisition is
    /// allowed to await, while every read stays synchronous.
    fn fetch_seed(&self, round: Round) -> BoxFuture<'_, bool>;

    /// The seed in force at `round`, if this node has it. Sync: both callers —
    /// the propose-side witness embed and the executor's re-canonicalisation —
    /// read it without an await.
    fn seed_for(&self, round: Round) -> Option<Seed>;

    /// Fires on every [`Self::record_seed`]. SINGLE CONSUMER: `notify_one` wakes
    /// exactly one waiter, so a second consumer of this `Arc` silently swallows
    /// wakes. Capture the `Arc` ONCE before the loop — the permit is
    /// object-scoped, so re-deriving the handle per iteration loses fills.
    fn seed_edge(&self) -> Arc<Notify>;

    /// Is randomness mandatory at `epoch`?
    ///
    /// CONSENSUS-AGREED DATA, not a local capability: every node of one network
    /// must answer identically, or the witness-required wire rule splits the
    /// network. This is the one operation [`absent`] answers TRUTHFULLY rather
    /// than negatively.
    ///
    /// The default body IS that agreed answer — the single deterministic
    /// bootstrap edge every node of this network compiles in. Override it only
    /// if your module owns a DIFFERENT bootstrap edge (today: the test provider,
    /// which parametrises it); never override it to report a local capability,
    /// because that is exactly the split this operation exists to prevent.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    /// Check a block's parent-seed witness under the key in force at
    /// `parent_epoch`. Sync: it is consumed inside the verify poll loop that
    /// shares one tick budget with the result gate, and an await there would
    /// corrupt the tick accounting.
    fn check_witness(&self, parent_epoch: u64, seed: &Seed) -> WitnessCheck;

    /// Can this node participate at `epoch` at all? See [`ShareProbe`].
    fn share_probe(&self, epoch: Epoch) -> ShareProbe;

    /// May this node sign at `epoch`, and with what scheme?
    ///
    /// Sync, and that is load-bearing rather than incidental: the implementation
    /// publishes this node's key internally before returning, and the returned
    /// scheme is a required input of the engine spawn — so
    /// publish-happens-before-spawn is a DATA DEPENDENCY, not an ordering
    /// convention a refactor can silently break.
    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict;

    /// Fires when this node's ability to participate may have changed. Single
    /// consumer; same capture-once rule as [`Self::seed_edge`].
    fn participation_edge(&self) -> Arc<Notify>;

    /// The beacon's threshold face for `epoch`, for a scheme that only VERIFIES.
    ///
    /// Sync, and it must stay so: the returned oracle is consulted inline from
    /// the simplex batcher, so anything it needs has to be resolvable without an
    /// await. Acquiring what it reads is [`Self::ensure_key`]'s job, off-path.
    ///
    /// CONTRACT, binding on every implementation: this MUST answer `None` where
    /// [`Self::mandatory_at`] is false. An oracle's PRESENCE is what tells
    /// `verify_certificate` "this epoch is beacon-active", so one attached to a
    /// pre-beacon epoch rejects every LEGAL seedless certificate there. This is
    /// the rule the write-once seed pin used to carry, at the same source and for
    /// the same reason — except an oracle is no longer write-once, so the damage
    /// is no longer permanent.
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>>;

    /// Try to make `epoch`'s group key locally resolvable, and report whether it
    /// now is. ACQUISITION, not verification: the value never leaves the beacon
    /// — what reads it is the oracle above.
    ///
    /// Async because [`PinEffort::Thorough`] may spend one bounded peer
    /// round-trip. [`PinEffort::Local`] is contractually network-free — it is
    /// the variant a vote path or a per-certificate path may call.
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool>;

    /// Fires when a key landed that [`Self::ensure_key`] did not have before.
    /// Single consumer; same capture-once rule as [`Self::seed_edge`].
    fn key_edge(&self) -> Arc<Notify>;

    /// The core reconciled `reconciled` while its highest registered epoch is
    /// `entered_frontier`.
    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch);

    /// The cert-inlet ingested a verified certificate for `epoch`.
    ///
    /// A SECOND observation and not a redundancy: the inlet prunes from its own
    /// live-upstream frontier, the epoch manager from its entered frontier, and
    /// each is the right one for its caller. Collapsing them would make the
    /// implementation guess.
    fn observe_cert(&self, epoch: u64);
}

/// The total, permanently-negative provider.
///
/// A light node and `--cert-follow` run with no beacon material at all, and "not
/// yet" is the wrong answer for them: the rungs are absent for the LIFE OF THE
/// PROCESS, not until something resolves. This turns that contract from prose
/// into a type.
///
/// Takes a metrics context because registration is context-scoped and commonware
/// prefixes each family with the context's label path — a context-free
/// constructor could not register anything.
pub fn absent(ctx: &impl Metrics) -> Arc<dyn Randomness> {
    let metrics = BeaconMetrics::default();
    metrics.register(ctx);
    absent_unregistered()
}

/// The same negative provider without the registration.
///
/// For instances that publish nothing by design — every test, and a
/// [`crate::application::FluentApp`] before its provider is attached. Splitting
/// the registration out is what lets those exist without a metrics context and
/// without silently claiming a second owner for a family.
pub(crate) fn absent_unregistered() -> Arc<dyn Randomness> {
    Arc::new(Absent {
        idle: Arc::new(Notify::new()),
    })
}

/// THE ACCEPTANCE TEST'S IMPLEMENTATION: randomness with no DKG, no agreement
/// plane, no network and no store.
///
/// The ticket's bar is that an implementation computing the seed from a hash be
/// substitutable here without a single change above this file. This is that
/// implementation, and its existence is the proof — if the surface still leaked
/// a beacon concept, this type could not satisfy it.
///
/// **It hashes into a SHARING, not into each σ, and that is forced rather than
/// stylistic.** `BlsScheme` is a concrete `CombinedScheme`: a seed is a BLS
/// threshold signature that real committee members recover from real partials,
/// so a mock that returned `H(round)` as a "signature" would be rejected by
/// `verify_seed` on the very first vote. What CAN be made static is the DKG's
/// output: seed a deterministic RNG from `H(chain_id ‖ epoch ‖ committee)`, deal
/// once, and every node of that committee derives the identical `PK_epoch` and
/// share set without exchanging a message. Everything downstream is then the
/// SHIPPED crypto, which is what makes this a substitute rather than a bypass.
///
/// Being able to deal means it can also produce σ for any round on demand — sign
/// `t` partials and recover — so `record_seed` is a no-op and `seed_for` needs no
/// memo. That is the "no store" half of the requirement.
#[cfg(test)]
pub(crate) struct StaticRandomness {
    chain_id: u64,
    namespace: Vec<u8>,
    /// The committee this implementation is randomness FOR. A static
    /// implementation legitimately knows it: that is the whole input its `PK` is
    /// derived from, and the surface's own operations do not carry a snapshot on
    /// every call.
    snap: ValidatorSetSnapshot,
    idle: Arc<Notify>,
}

#[cfg(test)]
impl StaticRandomness {
    pub(crate) fn build(chain_id: u64, snap: ValidatorSetSnapshot) -> Arc<dyn Randomness> {
        Arc::new(Self {
            chain_id,
            namespace: fluentbase_bls::beacon::seed_namespace(&fluentbase_bls::fluent_namespace(
                chain_id,
            )),
            snap,
            idle: Arc::new(Notify::new()),
        })
    }

    /// The whole of this implementation's "DKG": one deterministic deal, keyed on
    /// data every node of the epoch already agrees on.
    fn deal(
        &self,
        epoch: u64,
    ) -> (
        commonware_cryptography::bls12381::primitives::sharing::Sharing<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
        >,
        Vec<commonware_cryptography::bls12381::primitives::group::Share>,
    ) {
        use commonware_cryptography::{
            bls12381::{dkg::deal_anonymous, primitives::variant::MinSig},
            Hasher as _, Sha256,
        };
        use rand_core::SeedableRng as _;

        let mut h = Sha256::new();
        h.update(&self.chain_id.to_be_bytes());
        h.update(&epoch.to_be_bytes());
        // Sorted, so the digest does not depend on snapshot iteration order — the
        // same reason `constant_fallback_seed` sorts.
        let mut peers: Vec<&[u8]> = self
            .snap
            .validators
            .iter()
            .map(|v| v.keys.peer_pubkey.as_ref())
            .collect();
        peers.sort_unstable();
        for p in &peers {
            h.update(p);
        }
        let digest = h.finalize();
        let mut seed = [0u8; 32];
        seed.copy_from_slice(digest.as_ref());
        let mut rng = rand_08::rngs::StdRng::from_seed(seed);
        let n = commonware_utils::NZU32!(u32::try_from(peers.len()).expect("committee fits u32"));
        deal_anonymous::<MinSig, commonware_utils::N3f1>(&mut rng, Default::default(), n)
    }

    /// A [`SeedOracle`] over the epoch's DERIVED sharing. Deriving instead of
    /// storing is what makes this provider static; the oracle contract is the
    /// same one the plane's implements.
    fn dealt_oracle(&self, epoch: u64, share: Option<Share>) -> Arc<dyn SeedOracle> {
        Arc::new(DealtOracle {
            sharing: self.deal(epoch).0,
            share,
            namespace: self.namespace.clone(),
        })
    }

    /// σ for a round, produced the way the network produces it — `t` real
    /// partials, really recovered — just without anyone sending them.
    fn sigma(&self, round: Round) -> BlsSignature {
        let (sharing, shares) = self.deal(round.epoch().get());
        let t = sharing.required::<commonware_utils::N3f1>() as usize;
        let partials: Vec<_> = shares[..t]
            .iter()
            .map(|sh| fluentbase_bls::beacon::sign_seed_partial(sh, &self.namespace, round))
            .collect();
        fluentbase_bls::beacon::recover_seed::<commonware_utils::N3f1>(&sharing, &partials)
            .expect("t honest partials recover")
    }
}

#[cfg(test)]
/// A [`SeedOracle`] over one fixed sharing — the material a scheme used to hold
/// itself. Test-only: production reads the live ceremony store through
/// [`BeaconOracle`]. Shared with the cert-inlet's beacon fixtures so there is one
/// test oracle rather than one per test module.
#[derive(Debug)]
pub(crate) struct DealtOracle {
    pub(crate) sharing: Sharing<MinSig>,
    pub(crate) share: Option<Share>,
    pub(crate) namespace: Vec<u8>,
}

#[cfg(test)]
impl SeedOracle for DealtOracle {
    fn sign_partial(&self, round: Round) -> Option<BlsSignature> {
        let share = self.share.as_ref()?;
        Some(beacon_bls::sign_seed_partial(share, &self.namespace, round).value)
    }

    fn verify_partial(&self, round: Round, index: Participant, value: &BlsSignature) -> bool {
        use commonware_cryptography::bls12381::primitives::variant::PartialSignature;
        beacon_bls::verify_seed_partial(
            &self.sharing,
            &self.namespace,
            round,
            &PartialSignature::<MinSig> {
                index,
                value: *value,
            },
        )
    }

    fn recover(
        &self,
        partials: &[(Participant, BlsSignature)],
        threshold: u32,
    ) -> Option<BlsSignature> {
        use commonware_cryptography::bls12381::primitives::variant::PartialSignature;
        let partials: Vec<PartialSignature<MinSig>> = partials
            .iter()
            .map(|&(index, value)| PartialSignature::<MinSig> { index, value })
            .collect();
        beacon_bls::recover_seed_with_threshold(&self.sharing, &partials, threshold).ok()
    }

    fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck {
        if beacon_bls::verify_seed(self.sharing.public(), &self.namespace, round, seed) {
            SeedCheck::Valid
        } else {
            SeedCheck::Invalid
        }
    }
}

#[cfg(test)]
impl Randomness for StaticRandomness {
    /// Nothing to record: σ is recomputable for any round, so there is no memo
    /// that could go stale and none that has to be fed.
    fn record_seed(&self, _verified: VerifiedSeed) {}

    fn quarantine_seed(&self, _round: Round, _seed: BlsSignature) {}

    fn fetch_seed(&self, _round: Round) -> BoxFuture<'_, bool> {
        Box::pin(std::future::ready(false))
    }

    /// No key store to judge provenance with, so a failure proves nothing.
    fn on_invalid_seed(&self, _epoch: u64) -> InvalidSeed {
        InvalidSeed::Quarantine
    }

    fn seed_for(&self, round: Round) -> Option<Seed> {
        Some(Seed {
            target_round: round,
            signature: self.sigma(round),
        })
    }

    fn seed_edge(&self) -> Arc<Notify> {
        // Never fires, and correctly so: a consumer waiting for "a seed landed"
        // is waiting for something that was already true.
        self.idle.clone()
    }

    fn check_witness(&self, parent_epoch: u64, seed: &Seed) -> WitnessCheck {
        let (sharing, _) = self.deal(parent_epoch);
        if fluentbase_bls::beacon::verify_seed(
            sharing.public(),
            &self.namespace,
            seed.target_round,
            &seed.signature,
        ) {
            WitnessCheck::Valid
        } else {
            WitnessCheck::Invalid
        }
    }

    /// Never withheld: this implementation cannot fail to hold a share.
    fn share_probe(&self, _epoch: Epoch) -> ShareProbe {
        ShareProbe::Ready
    }

    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        let committee = match crate::scheme::epoch_committee_from_snapshot(snap) {
            Ok(c) => c,
            Err(e) => return SignerVerdict::InvalidCommittee(e),
        };
        let ns = fluentbase_bls::fluent_namespace(self.chain_id);
        let shares = self.deal(epoch.get()).1;
        // The share index must equal this node's CONSENSUS participant index —
        // the oracle refuses to sign otherwise. Ask the vote scheme where this
        // member sits rather than assuming the fixture's ordering.
        let seat = {
            use commonware_cryptography::certificate::Scheme as _;
            fluentbase_bls::scheme::build_signer(
                &ns,
                committee.bimap.clone(),
                keypair,
                epoch.get(),
                None,
            )
            .and_then(|s| s.me())
        };
        let Some(seat) = seat else {
            // Mirrors `PlaneRandomness`: a rotated-out node still verifies the
            // epoch's seed slot. A fixture that handed `None` here would model
            // the defect instead of the contract.
            return SignerVerdict::RotatedKey(fluentbase_bls::scheme::build_verifier(
                &ns,
                committee.bimap,
                epoch.get(),
                self.oracle_for(epoch.get()),
            ));
        };
        // UNCONDITIONAL, unlike this type's own `oracle_for` (which refuses below
        // `DETERMINISTIC_BOOTSTRAP_EPOCH`) and unlike `PlaneRandomness`'s `Signs`
        // arm (which inherits that refusal through `material`). This fixture can
        // DERIVE material for any epoch, so nothing here fails closed for it.
        //
        // Left unconditional deliberately, not by omission: every driver of this
        // arm signs at a beacon-active epoch, and gating it would make the fixture
        // silently answer `Withheld`-shaped nonsense if one ever did not, where
        // today it would fail loudly on the seedless-certificate assertion. But
        // this type is documented as the proof that the surface does not leak, so
        // it IS what a future reader copies — if you add a pre-beacon driver, gate
        // this on `mandatory_at` rather than discovering it through a red.
        let oracle = self.dealt_oracle(epoch.get(), Some(shares[usize::from(seat)].clone()));
        match fluentbase_bls::scheme::build_signer(
            &ns,
            committee.bimap,
            keypair,
            epoch.get(),
            Some(oracle),
        ) {
            Some(scheme) => SignerVerdict::Signs(scheme),
            None => unreachable!("the seat lookup above already proved membership"),
        }
    }

    fn participation_edge(&self) -> Arc<Notify> {
        self.idle.clone()
    }

    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        // The contract's pre-beacon refusal binds every implementation, not just
        // the one wired to a real plane — and being able to DERIVE a key for any
        // epoch is exactly what would make this one violate it silently.
        self.mandatory_at(epoch)
            .then(|| self.dealt_oracle(epoch, None))
    }

    fn ensure_key(&self, epoch: u64, _effort: PinEffort) -> BoxFuture<'_, bool> {
        // Both efforts answer identically: there is no network rung to spend, and
        // a derivable key is always already resolvable.
        Box::pin(async move { self.mandatory_at(epoch) })
    }

    fn key_edge(&self) -> Arc<Notify> {
        self.idle.clone()
    }

    fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

    fn observe_cert(&self, _epoch: u64) {}
}

/// An empty ceremony store, for the two entry points below: neither carries
/// local DKG material, so their oracles answer from the key store alone.
fn keyless_ceremony() -> CeremonyStore {
    Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()))
}

/// A `dkgQual` history with no bit set anywhere. Paired with
/// [`keyless_ceremony`]: with no mint stored, the carry walk has nothing to
/// find whatever it answers.
fn no_mint() -> DkgQualFor {
    Arc::new(|_| Some(false))
}

/// A provider over a seed store alone, for tests that exercise the executor's
/// two seed operations against a REAL store rather than canned answers.
///
/// Deliberately not `#[cfg(test)]`-gated the way [`testing::Canned`] is: it
/// builds the same [`PlaneRandomness`] production uses, so a test written
/// against it is testing the shipped `seed_for` / `seed_edge`, not a stub that
/// happens to agree with them today.
pub fn for_seeds(seeds: super::certify::SeedStore) -> Arc<dyn Randomness> {
    PlaneRandomness::build(PlaneRandomnessConfig {
        seeds,
        keys: BeaconKeys::new(),
        verify: None,
        resolver: Arc::new(|_| BeaconResolve::Absent),
        ceremony: keyless_ceremony(),
        dkg_qual: no_mint(),
        held: None,
        pull: None,
        pull_seed: None,
        participation: Arc::new(Notify::new()),
        metrics: BeaconMetrics::default(),
        chain_id: 0,
    })
}

/// A provider over a key store alone. **TEST ENTRY POINT.**
///
/// Not `#[cfg(test)]`-gated for one reason: a test in the NODE crate needs it,
/// and a cross-crate caller cannot see this crate's test items. That is the whole
/// justification, and it is written down because the previous doc here read like a
/// production contract — it opened "`keys` MUST be the layer's existing store,
/// never a fresh one", which described the cert-inlet mount as it stood before the
/// stores moved into `beacon::build`. That mount is gone: both inlets take the
/// layer's own provider now, and this constructor has **no production caller**.
/// A doc that still issues instructions for a vanished wiring is how someone
/// reconnects a test seam as a production one.
///
/// Built on the same [`PlaneRandomness`] as everything else, so a test written
/// against it exercises the shipped ladder rather than a stub.
pub fn for_keys(keys: BeaconKeys, held: Option<super::keys::AgreedKeys>) -> Arc<dyn Randomness> {
    PlaneRandomness::build(PlaneRandomnessConfig {
        seeds: super::certify::SeedStore::new(),
        keys,
        verify: None,
        resolver: Arc::new(|_| BeaconResolve::Absent),
        ceremony: keyless_ceremony(),
        dkg_qual: no_mint(),
        held,
        pull: None,
        pull_seed: None,
        participation: Arc::new(Notify::new()),
        metrics: BeaconMetrics::default(),
        // Reaches `build_signer` only, which an ingress path never calls.
        chain_id: 0,
    })
}

/// Holds no store, and that is the honest shape. It carried a `BeaconKeys` whose
/// doc said the follower "keeps ONE key store for its ladder and its retention" —
/// but this provider's `ensure_key` never reads a store and no writer can reach one,
/// so the field's only user was its own retention call: pruning a map nothing
/// fills, on behalf of a ladder with no rungs. Removed rather than kept for
/// symmetry, because the doc read as an instruction.
struct Absent {
    idle: Arc<Notify>,
}

impl Randomness for Absent {
    fn record_seed(&self, _verified: VerifiedSeed) {}

    fn quarantine_seed(&self, _round: Round, _seed: BlsSignature) {}

    fn fetch_seed(&self, _round: Round) -> BoxFuture<'_, bool> {
        Box::pin(std::future::ready(false))
    }

    /// No key store to judge provenance with, so a failure proves nothing.
    fn on_invalid_seed(&self, _epoch: u64) -> InvalidSeed {
        InvalidSeed::Quarantine
    }

    fn seed_for(&self, _round: Round) -> Option<Seed> {
        None
    }

    fn seed_edge(&self) -> Arc<Notify> {
        self.idle.clone()
    }

    fn check_witness(&self, _parent_epoch: u64, _seed: &Seed) -> WitnessCheck {
        WitnessCheck::NoKey
    }

    fn share_probe(&self, _epoch: Epoch) -> ShareProbe {
        ShareProbe::Withheld(WithheldReason::NoUsableShare)
    }

    fn signer_scheme(
        &self,
        _epoch: Epoch,
        _snap: &ValidatorSetSnapshot,
        _keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        SignerVerdict::Withheld(WithheldReason::NoUsableShare)
    }

    fn participation_edge(&self) -> Arc<Notify> {
        self.idle.clone()
    }

    /// No oracle at any epoch, permanently: with nothing to read, one would only
    /// turn every legal seedless certificate into a rejection.
    fn oracle_for(&self, _epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        None
    }

    fn ensure_key(&self, _epoch: u64, _effort: PinEffort) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn key_edge(&self) -> Arc<Notify> {
        self.idle.clone()
    }

    fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

    /// Nothing to prune: there is no store, because nothing could ever fill one.
    fn observe_cert(&self, _epoch: u64) {}
}

/// Test provider. One place for every migrated test to get a [`Randomness`],
/// so call sites stop inventing a stub each.
///
/// It is also the SPY: it records the `(epoch, effort)` of every
/// [`Randomness::ensure_key`] call, which is what lets the repair sweep's tests
/// assert which effort was spent rather than only that a key appeared.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::beacon::{keys::KeySource, oracle::KeyOnlyOracle};
    use std::{collections::BTreeMap, sync::Mutex};

    #[derive(Default)]
    pub(crate) struct Canned {
        /// Canned `PK_epoch` values, reachable through [`Randomness::oracle_for`]
        /// exactly as a real one is: a plain store the oracle reads.
        keys: BeaconKeys,
        witness: Option<WitnessCheck>,
        seeds: BTreeMap<Round, Seed>,
        bootstrap: u64,
        efforts: Mutex<Vec<(u64, PinEffort)>>,
        idle: Arc<Notify>,
        /// A real store, so a test can assert WHERE a captured σ landed —
        /// served or held — instead of only that the call happened.
        store: crate::beacon::certify::SeedStore,
        /// Must match the namespace the fixture signed under, or every σ this
        /// provider is asked about is `Invalid` rather than `Valid`.
        seed_namespace: Vec<u8>,
    }

    impl Canned {
        pub(crate) fn new() -> Self {
            Self {
                bootstrap: super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH,
                idle: Arc::new(Notify::new()),
                ..Default::default()
            }
        }

        pub(crate) fn with_pin(self, epoch: u64, pk: GroupPublic) -> Self {
            self.keys.set_pk(epoch, pk, KeySource::Agreed);
            self
        }

        pub(crate) fn with_seed_namespace(mut self, namespace: Vec<u8>) -> Self {
            self.seed_namespace = namespace;
            self
        }

        pub(crate) fn store(&self) -> &crate::beacon::certify::SeedStore {
            &self.store
        }

        /// Every `ensure_key` this provider answered, in call order.
        pub(crate) fn efforts(&self) -> Vec<(u64, PinEffort)> {
            self.efforts.lock().expect("efforts lock").clone()
        }
    }

    impl Randomness for Canned {
        fn record_seed(&self, verified: VerifiedSeed) {
            self.store.record(verified);
        }

        fn quarantine_seed(&self, round: Round, seed: BlsSignature) {
            self.store.quarantine(round, seed);
        }

        fn fetch_seed(&self, _round: Round) -> BoxFuture<'_, bool> {
            Box::pin(std::future::ready(false))
        }

        fn on_invalid_seed(&self, epoch: u64) -> InvalidSeed {
            self.keys.on_invalid_seed(epoch)
        }

        fn seed_for(&self, round: Round) -> Option<Seed> {
            self.seeds.get(&round).cloned()
        }

        fn seed_edge(&self) -> Arc<Notify> {
            self.idle.clone()
        }

        fn mandatory_at(&self, epoch: u64) -> bool {
            epoch >= self.bootstrap
        }

        fn check_witness(&self, _parent_epoch: u64, _seed: &Seed) -> WitnessCheck {
            match self.witness {
                Some(WitnessCheck::Valid) => WitnessCheck::Valid,
                Some(WitnessCheck::Invalid) => WitnessCheck::Invalid,
                Some(WitnessCheck::Undecided) => WitnessCheck::Undecided,
                Some(WitnessCheck::NoKey) | None => WitnessCheck::NoKey,
            }
        }

        fn share_probe(&self, _epoch: Epoch) -> ShareProbe {
            ShareProbe::Withheld(WithheldReason::NoUsableShare)
        }

        fn signer_scheme(
            &self,
            _epoch: Epoch,
            _snap: &ValidatorSetSnapshot,
            _keypair: &ValidatorBlsKeypair,
        ) -> SignerVerdict {
            SignerVerdict::Withheld(WithheldReason::NoUsableShare)
        }

        fn participation_edge(&self) -> Arc<Notify> {
            self.idle.clone()
        }

        fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
            self.mandatory_at(epoch).then(|| {
                Arc::new(KeyOnlyOracle {
                    epoch,
                    keys: self.keys.clone(),
                    namespace: self.seed_namespace.clone(),
                    metrics: BeaconMetrics::default(),
                }) as Arc<dyn SeedOracle>
            })
        }

        fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
            self.efforts
                .lock()
                .expect("efforts lock")
                .push((epoch, effort));
            let known = self.keys.cached_only(epoch).is_some();
            Box::pin(async move { known })
        }

        fn key_edge(&self) -> Arc<Notify> {
            self.idle.clone()
        }

        fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

        fn observe_cert(&self, _epoch: u64) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_math::algebra::Additive as _;
    use commonware_runtime::{deterministic::Runner, Runner as _};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    #[test]
    fn the_absent_provider_answers_negatively_except_on_the_network_wide_rule() {
        let runner = Runner::default();
        runner.start(|ctx| async move {
            let r = absent(&ctx);

            assert_eq!(
                r.check_witness(
                    9,
                    &crate::beacon::seed::Seed {
                        target_round: Round::new(
                            Epoch::new(9),
                            commonware_consensus::types::View::new(1)
                        ),
                        signature: BlsSignature::zero(),
                    }
                ),
                WitnessCheck::NoKey
            );
            assert_eq!(
                r.share_probe(Epoch::new(9)),
                ShareProbe::Withheld(WithheldReason::NoUsableShare)
            );
            assert!(r
                .seed_for(Round::new(
                    Epoch::new(9),
                    commonware_consensus::types::View::new(1)
                ))
                .is_none());
            assert!(r.oracle_for(9).is_none());
            assert!(!r.ensure_key(9, PinEffort::Local).await);
            assert!(!r.ensure_key(9, PinEffort::Thorough).await);

            // The one truthful answer: the witness-required rule is network-wide
            // agreed data, so answering it negatively would split the network
            // rather than degrade this node.
            assert!(!r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1));
            assert!(r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH));
        });
    }

    /// W1's suppression rule, and the reason it is CONDITIONAL.
    ///
    /// Where the agreement plane has published a key, a locally reconstructed one
    /// has nothing to add and W1 stands down. Where it has not — every stable
    /// carry-forward epoch, which runs no agreement instance at all — W1 must still
    /// publish, or ladder rung 1 stops answering for epochs this node signed,
    /// `repair_keyless_schemes` (which consults no local DKG material precisely
    /// BECAUSE W1 fills the store) never resolves them, and their oracles keep
    /// answering `NoKey` — vote-only cert admission for the life of the process.
    #[test]
    fn w1_defers_to_an_agreed_key_and_publishes_where_there_is_none() {
        let mine = {
            let mut rng = StdRng::seed_from_u64(0x5EED);
            let (sharing, _) =
                commonware_cryptography::bls12381::dkg::deal_anonymous::<
                    commonware_cryptography::bls12381::primitives::variant::MinSig,
                    commonware_utils::N3f1,
                >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
            *sharing.public()
        };
        let theirs = {
            let mut rng = StdRng::seed_from_u64(0xB0B);
            let (sharing, _) =
                commonware_cryptography::bls12381::dkg::deal_anonymous::<
                    commonware_cryptography::bls12381::primitives::variant::MinSig,
                    commonware_utils::N3f1,
                >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
            *sharing.public()
        };
        assert_ne!(mine, theirs, "the fixture must produce two distinct keys");

        assert_eq!(own_key_publication(None, mine), OwnKeyPublication::Publish);
        assert_eq!(
            own_key_publication(Some(mine), mine),
            OwnKeyPublication::DeferAgreeing
        );
        assert_eq!(
            own_key_publication(Some(theirs), mine),
            OwnKeyPublication::DeferDiverging(theirs)
        );
    }

    /// A committee snapshot with real BLS keypairs, plus DKG material for it.
    /// Returns the keypair at index 0 (a member) and one that is NOT in the
    /// committee, so both the signing and the rotated-out arms are reachable.
    fn signer_fixture(
        epoch: Epoch,
    ) -> (
        ValidatorSetSnapshot,
        ValidatorBlsKeypair,
        ValidatorBlsKeypair,
        BeaconKey,
    ) {
        use alloy_primitives::{Address, B256};
        use commonware_codec::DecodeExt as _;
        use commonware_cryptography::{
            bls12381::{dkg::deal_anonymous, primitives::variant::MinSig},
            ed25519::PrivateKey as Ed25519PrivateKey,
            Signer as _,
        };
        use commonware_math::algebra::Random as _;
        use fluentbase_bls::BlsPubkey;
        use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};

        let mut keypairs = Vec::new();
        let validators = (0..4u8)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(0x5EED + i as u64);
                let kp = ValidatorBlsKeypair::generate(&mut rng);
                let bls_pubkey = BlsPubkey::decode(kp.public_bytes().as_slice()).unwrap();
                keypairs.push(kp);
                ValidatorWithKeys {
                    address: Address::repeat_byte(i),
                    keys: ConsensusKeys {
                        bls_pubkey,
                        peer_pubkey: Ed25519PrivateKey::random(&mut rng).public_key(),
                        activation_epoch: 1,
                    },
                    tombstoned: false,
                }
            })
            .collect();
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x22),
            block_number: 7,
            epoch: epoch.get(),
            validators,
            weights: None,
        };

        let mut rng = StdRng::seed_from_u64(0xDEA1);
        let (sharing, shares) = deal_anonymous::<MinSig, commonware_utils::N3f1>(
            &mut rng,
            Default::default(),
            commonware_utils::NZU32!(4),
        );
        // `CombinedScheme::new` asserts the share's index equals this node's
        // CONSENSUS participant index, and that index comes from the committee's
        // sorted order — not from the order the fixture generated the keypairs
        // in. Ask the vote scheme where this member actually sits.
        let member = keypairs.remove(0);
        let committee = crate::scheme::epoch_committee_from_snapshot(&snap).expect("committee");
        let seat: usize = {
            use commonware_cryptography::certificate::Scheme as _;
            fluentbase_bls::scheme::build_signer(
                &fluentbase_bls::fluent_namespace(1),
                committee.bimap,
                &member,
                snap.epoch,
                None,
            )
            .and_then(|s| s.me())
            .expect("the fixture's member is in its own committee")
            .into()
        };
        let material: BeaconKey = (
            sharing,
            Some(shares[seat].clone()),
            b"test-seed-ns".to_vec(),
        );

        let outsider = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(0xDEAD));
        (snap, member, outsider, material)
    }

    fn provider_over(keys: BeaconKeys, material: BeaconKey) -> Arc<dyn Randomness> {
        PlaneRandomness::build(PlaneRandomnessConfig {
            seeds: super::super::certify::SeedStore::new(),
            keys,
            verify: None,
            resolver: Arc::new(move |_| BeaconResolve::Key(material.clone())),
            ceremony: keyless_ceremony(),
            dkg_qual: no_mint(),
            held: None,
            pull: None,
            pull_seed: None,
            participation: Arc::new(Notify::new()),
            metrics: BeaconMetrics::default(),
            chain_id: 1,
        })
    }

    /// W1's ordering guarantee, in the only form that survives the move: the
    /// caller CANNOT hold a signing scheme for an epoch whose `PK_epoch` is not
    /// already in the key store. The core used to assert this from outside with a
    /// `debug_assert` in `spawn_engine`; it can no longer see the store, so the
    /// property has to be provable from the surface alone.
    ///
    /// The engine cannot vote before it is spawned and cannot be spawned without
    /// this scheme, so "published before any vote" follows from this one
    /// assertion — no ordering convention a refactor can quietly drop.
    #[test]
    fn a_signing_scheme_is_never_handed_out_before_the_epoch_key_is_published() {
        let epoch = Epoch::new(9);
        let (snap, member, _outsider, material) = signer_fixture(epoch);
        let keys = BeaconKeys::new();
        let expected = *material.0.public();
        let randomness = provider_over(keys.clone(), material);

        assert!(
            keys.cached_only(epoch.get()).is_none(),
            "fixture precondition: nothing published yet"
        );
        let verdict = randomness.signer_scheme(epoch, &snap, &member);

        assert!(
            matches!(verdict, SignerVerdict::Signs(_)),
            "a committee member holding verifying DKG material signs"
        );
        assert_eq!(
            keys.cached_only(epoch.get()),
            Some(expected),
            "the scheme came back only after PK_epoch was in the store"
        );
    }

    /// The misconfiguration safety net: a keypair the committee does not contain
    /// gets a verify-only scheme, not a withhold. The caller still spawns on it
    /// (and aborts on its next reconcile), which is the behaviour the engine used
    /// to implement for itself.
    #[test]
    fn a_keypair_outside_the_committee_gets_a_verify_only_scheme() {
        let epoch = Epoch::new(9);
        let (snap, _member, outsider, material) = signer_fixture(epoch);
        let randomness = provider_over(BeaconKeys::new(), material);

        assert!(matches!(
            randomness.signer_scheme(epoch, &snap, &outsider),
            SignerVerdict::RotatedKey(_)
        ));
    }

    /// A ROTATED-OUT NODE STILL JUDGES THE EPOCH'S SEED SLOT, and this is a
    /// producer obligation that no downstream guard can cover for.
    ///
    /// `EpochSchemeProvider::register`'s three refusals all live under
    /// `Entry::Occupied`; a vacant slot is inserted unconditionally. On a clean
    /// live-epoch path — `share_probe` `Ready`, the boundary block present, so
    /// neither of `reconcile_roles`' early exits soft-enters — this verdict's
    /// scheme is the epoch's FIRST registration. Land an oracle-less one there
    /// and it stays for the whole retention window, admitting every epoch-E
    /// certificate whose seed slot was cleared, while `repair_keyless_schemes`
    /// resolves the key and reports the epoch as upgraded.
    ///
    /// The earlier "exact parity with today's `beacon: None`" reasoning held only
    /// for the attestation arm. On the certificate arm the pre-FLU-1202 scheme
    /// was REPAIRABLE — `apply_pin` attached `PK_epoch` to it later — and nothing
    /// repairs a scheme now, so parity means reproducing the POST-pin state:
    /// verify-only, but seed-checking.
    ///
    /// Reds if the `RotatedKey` arm goes back to `None`.
    #[test]
    fn a_rotated_out_node_still_verifies_the_epochs_seed_slot() {
        use crate::outer::EpochSchemeProvider;
        use commonware_cryptography::certificate::Provider as _;

        let epoch = Epoch::new(9);
        let (snap, _member, outsider, material) = signer_fixture(epoch);
        let randomness = provider_over(BeaconKeys::new(), material);
        assert!(
            randomness.mandatory_at(epoch.get()),
            "fixture precondition: the epoch is beacon-active, so an oracle is \
             owed here"
        );

        let SignerVerdict::RotatedKey(scheme) = randomness.signer_scheme(epoch, &snap, &outsider)
        else {
            panic!("a keypair outside the committee gets the RotatedKey verdict");
        };
        assert!(
            scheme.is_beacon_active(),
            "the misconfiguration safety net must still carry the epoch's oracle"
        );

        // The vacant-slot path, spelled out because it is the reason the
        // assertion above cannot be moved into the registry: nothing here checks
        // anything, so whatever the producer emitted is what the epoch gets.
        let provider = EpochSchemeProvider::new();
        provider.register(epoch, scheme);
        assert!(
            provider
                .scoped(epoch)
                .expect("registered")
                .is_beacon_active(),
            "a vacant slot is inserted unconditionally — the registry cannot \
             tell that this epoch is beacon-active, so the producer must be \
             right on first insert"
        );
    }

    /// Each withhold reason owns a different metric family, so collapsing two of
    /// them loses a signal the devnet smoke scrapes. A share that does not verify
    /// against its own sharing must report `BadShare`, never the value gate's
    /// `KeyDivergence`.
    #[test]
    fn a_share_that_does_not_verify_reports_bad_share_and_not_key_divergence() {
        let epoch = Epoch::new(9);
        let (snap, member, _outsider, material) = signer_fixture(epoch);
        // Same sharing, a share from an unrelated deal: the index still matches,
        // the value does not lie on the polynomial.
        let mut rng = StdRng::seed_from_u64(0xBAD5);
        let (_, foreign) = commonware_cryptography::bls12381::dkg::deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            commonware_utils::N3f1,
        >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
        let tampered: BeaconKey = (material.0, Some(foreign[0].clone()), material.2);
        let randomness = provider_over(BeaconKeys::new(), tampered);

        assert!(matches!(
            randomness.signer_scheme(epoch, &snap, &member),
            SignerVerdict::Withheld(WithheldReason::BadShare)
        ));
    }

    /// THE ACCEPTANCE TEST for this ticket.
    ///
    /// Drives the whole randomness contract — propose (the seed a leader embeds),
    /// verify (the witness check a voter runs), the signing scheme, and the
    /// certificate pin — against an implementation that has no DKG, no agreement
    /// plane, no network and no store. If any of those operations still required
    /// a beacon concept, this type could not exist and this test could not
    /// compile.
    ///
    /// Not a mock returning canned answers: every value below is real BLS,
    /// produced by the shipped crypto. What is static is only the DEAL — see
    /// `StaticRandomness`'s doc for why hashing into a sharing is forced rather
    /// than chosen.
    #[test]
    fn the_whole_surface_is_satisfied_by_an_implementation_with_no_dkg_and_no_store() {
        let epoch = Epoch::new(9);
        let round = Round::new(epoch, commonware_consensus::types::View::new(4));
        let (snap, member, outsider, _unused) = signer_fixture(epoch);
        let r = StaticRandomness::build(1, snap.clone());

        // Propose: the leader has a seed for the round, with no one to receive it
        // from and nothing recorded.
        let seed = r
            .seed_for(round)
            .expect("a static implementation always has σ");
        assert_eq!(seed.target_round, round);

        // Verify: another node checks that seed under the key in force, and
        // agrees — the two halves derived the same PK from the same committee
        // without exchanging anything.
        assert_eq!(r.check_witness(epoch.get(), &seed), WitnessCheck::Valid);

        // A DIFFERENT round's σ must not verify as this one's. Without this the
        // Valid above would also pass for an implementation that ignored the
        // round entirely.
        let other = Round::new(epoch, commonware_consensus::types::View::new(5));
        let wrong = Seed {
            target_round: round,
            signature: r.seed_for(other).expect("σ").signature,
        };
        assert_eq!(r.check_witness(epoch.get(), &wrong), WitnessCheck::Invalid);

        // Participation and scheme construction.
        assert_eq!(r.share_probe(epoch), ShareProbe::Ready);
        assert!(matches!(
            r.signer_scheme(epoch, &snap, &member),
            SignerVerdict::Signs(_)
        ));
        assert!(
            matches!(
                r.signer_scheme(epoch, &snap, &outsider),
                SignerVerdict::RotatedKey(_)
            ),
            "the rotated-out safety net is part of the contract, not of the beacon"
        );

        // The certificate oracle, and the network-wide rule.
        assert!(
            r.oracle_for(epoch.get()).is_some(),
            "a beacon-active epoch must carry an oracle, or its certificates are \
             admitted on the multisig half alone"
        );
        let runner = Runner::default();
        runner.start(move |_| async move {
            assert!(
                r.ensure_key(epoch.get(), PinEffort::Local).await,
                "a static implementation can always derive the epoch key"
            );
            assert!(
                r.ensure_key(epoch.get(), PinEffort::Thorough).await,
                "both efforts answer from the same derivation"
            );
            assert!(r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH));
        });
    }

    /// The quorum-attested key is sampled ONCE per `promote_gates` call.
    ///
    /// It used to be read twice — value gate, then W1 — with a BLS pairing check
    /// between, while the agreement write-back writes that same shared map from
    /// another task. A disagreement between the two reads meant the gate had
    /// passed on the stale one, and W1 only warned: the node signed with material
    /// it had just been shown was divergent.
    ///
    /// **This test cannot reach that window, and saying so is the point.** With a
    /// single read the W1 divergence arm is structurally unreachable from here, so
    /// there is nothing left to stage; with two reads it was reachable only by
    /// mutating the store between them, which a single-threaded test cannot do
    /// without injecting a mutating store. An earlier version of this test claimed
    /// to cover the arm and did not — it passed through the value gate, verified
    /// by removing the arm's body and watching it stay green.
    ///
    /// What IS testable, and what this asserts, is the decision the single sample
    /// produces: a node whose material diverges from the agreed key is withheld.
    #[test]
    fn material_diverging_from_the_agreed_key_is_withheld_from_signing() {
        let epoch = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1);
        let (snap, member, _outsider, material) = signer_fixture(epoch);
        let mine = *material.0.public();

        let keys = BeaconKeys::new();
        let theirs = {
            let mut rng = StdRng::seed_from_u64(0xD1FF);
            let (sharing, _) =
                commonware_cryptography::bls12381::dkg::deal_anonymous::<
                    commonware_cryptography::bls12381::primitives::variant::MinSig,
                    commonware_utils::N3f1,
                >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
            *sharing.public()
        };
        assert_ne!(
            mine, theirs,
            "premise: the fixture produces two distinct keys"
        );
        keys.set_pk(epoch.get(), theirs, KeySource::Agreed);

        let r = PlaneRandomness::build(PlaneRandomnessConfig {
            seeds: super::super::certify::SeedStore::new(),
            keys: keys.clone(),
            verify: None,
            resolver: Arc::new(move |_| BeaconResolve::Key(material.clone())),
            ceremony: keyless_ceremony(),
            dkg_qual: no_mint(),
            held: None,
            pull: None,
            pull_seed: None,
            participation: Arc::new(Notify::new()),
            metrics: BeaconMetrics::default(),
            chain_id: 1,
        });

        assert!(
            matches!(
                r.signer_scheme(epoch, &snap, &member),
                SignerVerdict::Withheld(WithheldReason::KeyDivergence)
            ),
            "a node holding material that diverges from the agreed key must not sign with it"
        );
        // And it must not have overwritten the agreed key on its way out — the
        // store is the comparand every later resolve depends on.
        assert_eq!(
            keys.attested(epoch.get()),
            Some(theirs),
            "the agreed key survives the withheld attempt"
        );
    }

    /// The participation probe answers the DIVERGENCE question, not only the
    /// "do I hold material" one — and that is what lets a caller revisit a
    /// decision it already made.
    ///
    /// `signer_scheme` runs once, at the spawn. This probe runs on every reconcile
    /// edge, including while an engine is live. A quorum can attest an epoch key
    /// AFTER this node spawned; if it differs from the one this node
    /// reconstructed, every vote it casts carries a seed partial the rest of the
    /// committee rejects. Until the probe answered this, nothing revisited it.
    ///
    /// The verdict must also be STABLE, or it would flap an engine down and up:
    /// the comparand is the attested tier, which appears once and is never
    /// downgraded. The repeat below asserts that.
    #[test]
    fn the_participation_probe_reports_a_divergence_that_appears_after_the_spawn() {
        let epoch = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1);
        let (_snap, _member, _outsider, material) = signer_fixture(epoch);
        let mine = *material.0.public();
        let keys = BeaconKeys::new();
        let r = PlaneRandomness::build(PlaneRandomnessConfig {
            seeds: super::super::certify::SeedStore::new(),
            keys: keys.clone(),
            verify: None,
            resolver: Arc::new(move |_| BeaconResolve::Key(material.clone())),
            ceremony: keyless_ceremony(),
            dkg_qual: no_mint(),
            held: None,
            pull: None,
            pull_seed: None,
            participation: Arc::new(Notify::new()),
            metrics: BeaconMetrics::default(),
            chain_id: 1,
        });

        // Before the quorum speaks: this node participates.
        assert_eq!(
            r.share_probe(epoch),
            ShareProbe::Ready,
            "premise: with no attested key the node participates — otherwise the \
             verdict below would not be caused by the divergence"
        );

        // The quorum attests a DIFFERENT key, exactly as the write-back would.
        let theirs = {
            let mut rng = StdRng::seed_from_u64(0xD1FF);
            let (sharing, _) =
                commonware_cryptography::bls12381::dkg::deal_anonymous::<
                    commonware_cryptography::bls12381::primitives::variant::MinSig,
                    commonware_utils::N3f1,
                >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
            *sharing.public()
        };
        assert_ne!(mine, theirs);
        keys.set_pk(epoch.get(), theirs, KeySource::Agreed);

        assert_eq!(
            r.share_probe(epoch),
            ShareProbe::Withheld(WithheldReason::KeyDivergence)
        );
        assert_eq!(
            r.share_probe(epoch),
            ShareProbe::Withheld(WithheldReason::KeyDivergence),
            "the verdict is stable — an engine aborted on it must not be re-spawned next edge"
        );
    }

    /// The share gate must hold at BOTH samples, not just at the probe.
    ///
    /// `share_probe` runs before the caller's boundary-block lookup and
    /// `signer_scheme` after it, so the material can disappear in between — an
    /// agreement artifact landing makes the carry-divergence guard refuse it, or
    /// the ceremony-store prune drops the mint. Every guard inside
    /// `signer_scheme` is written `if let Some(material)`, so on `None` they all
    /// pass vacuously and `build_signer` returns a working PURE-MULTISIG scheme.
    ///
    /// The verdict for a beacon-active epoch would then be `Signs`, the node
    /// would spawn a participating engine, and every vote it cast would be
    /// rejected for the missing seed partial — subtracting itself from the quorum
    /// while believing it signs, with no demote counter moving.
    #[test]
    fn a_beacon_active_epoch_with_no_material_is_withheld_and_never_signs_seedless() {
        let epoch = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1);
        let (snap, member, ..) = signer_fixture(epoch);
        // A provider whose resolver answers `Absent` — the state reached when the
        // material vanishes between the two samples.
        let r = PlaneRandomness::build(PlaneRandomnessConfig {
            seeds: super::super::certify::SeedStore::new(),
            keys: BeaconKeys::new(),
            verify: None,
            resolver: Arc::new(|_| BeaconResolve::Absent),
            ceremony: keyless_ceremony(),
            dkg_qual: no_mint(),
            held: None,
            pull: None,
            pull_seed: None,
            participation: Arc::new(Notify::new()),
            metrics: BeaconMetrics::default(),
            chain_id: 1,
        });

        assert!(
            matches!(
                r.signer_scheme(epoch, &snap, &member),
                SignerVerdict::Withheld(WithheldReason::NoUsableShare)
            ),
            "a committee member with no DKG material must be withheld at a beacon-active epoch"
        );

        // The premise the assertion rests on: BELOW the bootstrap epoch the same
        // provider, same absent material, legitimately signs pure-multisig. Without
        // this the assertion above would also hold for a provider that refuses
        // everything.
        let pre = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        let (pre_snap, pre_member, ..) = signer_fixture(pre);
        assert!(
            matches!(
                r.signer_scheme(pre, &pre_snap, &pre_member),
                SignerVerdict::Signs(_)
            ),
            "a pre-beacon epoch has a legitimate seedless signer — the refusal is about \
             beacon-active epochs only"
        );
    }

    /// The provenance floor, on BOTH efforts.
    ///
    /// A wrong epoch key is terminal for that epoch on this node; its own W1/W3
    /// reconstruction is the one tier that can diverge from the network's key. So
    /// no effort takes it — the asymmetry (wrong key rejects everything, missing
    /// key merely degrades to multisig-only admission) only ever runs one way.
    ///
    /// This test previously asserted the OPPOSITE for `Local`, on purpose: the
    /// floor had been reverted there after it halved the devnet block rate, and
    /// the test stated that gap so it could not close by accident. It closed
    /// deliberately — `chain_key_epoch` is memoised now, so the fall-through the
    /// floor causes no longer walks the epoch range on every certificate.
    #[test]
    fn no_effort_takes_a_locally_reconstructed_key() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let epoch = 9u64;
            let local_only = BeaconKeys::new();
            local_only.set_pk(epoch, GroupPublic::zero(), KeySource::LocalDkg);
            let r = for_keys(local_only.clone(), None);

            assert!(
                local_only.cached_only(epoch).is_some(),
                "premise: the store DOES hold an entry — the verdicts below are about its TIER"
            );
            assert!(!r.ensure_key(epoch, PinEffort::Local).await);
            assert!(
                !r.ensure_key(epoch, PinEffort::Thorough).await,
                "the rule is about the TIER, not about how hard the caller looked"
            );

            // A tier the floor admits, to prove this is a FLOOR and not a blanket
            // refusal of the store rung — without it both assertions above would
            // hold for a provider that had simply stopped reading the store.
            let carried = BeaconKeys::new();
            carried.set_pk(epoch, GroupPublic::zero(), KeySource::Carried);
            let r = for_keys(carried, None);
            assert!(r.ensure_key(epoch, PinEffort::Local).await);
            assert!(r.ensure_key(epoch, PinEffort::Thorough).await);
        });
    }

    /// The beacon-active rule, at the source. An ORACLE tells
    /// `verify_certificate` "this epoch is beacon-active", so one on a PRE-beacon
    /// epoch rejects every legal seedless certificate there.
    ///
    /// The soft-enter path and the cert-inlet both take their oracle from
    /// `oracle_for`, so asserting the refusal here asserts it for all of them.
    #[test]
    fn no_oracle_and_no_key_for_an_epoch_where_randomness_is_not_mandatory() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let epoch = Epoch::new(9);
            let (snap, ..) = signer_fixture(epoch);
            let r = StaticRandomness::build(1, snap);
            let pre = super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1;

            // Premise: this provider CAN answer — otherwise the assertions below
            // hold for an implementation that answers nothing at all.
            let bootstrap = super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
            assert!(
                r.oracle_for(bootstrap).is_some(),
                "premise: the provider answers at a beacon-active epoch"
            );
            assert!(r.ensure_key(bootstrap, PinEffort::Local).await);

            assert!(r.oracle_for(pre).is_none());
            assert!(!r.ensure_key(pre, PinEffort::Local).await);
            assert!(
                !r.ensure_key(pre, PinEffort::Thorough).await,
                "the rule is about the EPOCH, not about how hard the caller looked"
            );
        });
    }

    /// The other half of the acceptance bar: substituting the implementation
    /// required no change ABOVE the surface.
    ///
    /// Asserted by where the type is NAMED, not by a diff. A diff against `HEAD`
    /// cannot express this claim — this ticket deliberately rewrites the core to
    /// create the surface, so of course files outside `beacon/` changed. What the
    /// bar is actually about is narrower and checkable: dropping a
    /// no-DKG-no-store implementation in needs no core file to know it exists.
    ///
    /// (The first version of this test DID diff against `HEAD`, with a pathspec
    /// that silently matched nothing because it was resolved against the manifest
    /// directory. Its own non-vacuity assertion is what caught it, which is the
    /// argument for writing that assertion.)
    #[test]
    fn no_file_outside_the_beacon_names_the_static_implementation() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut inside = 0usize;
        let mut outside: Vec<String> = Vec::new();

        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable source dir") {
                let path = entry.expect("readable entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("readable source file");
                if !text.contains("StaticRandomness") {
                    continue;
                }
                let rel = path.strip_prefix(&src).expect("under src").to_owned();
                if rel.starts_with("beacon") {
                    inside += 1;
                } else {
                    outside.push(rel.display().to_string());
                }
            }
        }

        // Non-vacuity: a typo in the needle, or a walk that visited nothing, would
        // make the real assertion below trivially true.
        assert!(
            inside > 0,
            "premise: the static implementation lives under beacon/ and must be found there"
        );
        assert!(
            outside.is_empty(),
            "a core file names the substitute implementation, so the surface still leaks: {outside:?}"
        );
    }

    /// The spy is what lets the repair sweep's tests assert WHICH effort was
    /// spent rather than only that a key appeared. Prove it records here, before
    /// anything depends on it.
    #[test]
    fn the_test_provider_records_every_key_effort_it_was_asked_for() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let canned = testing::Canned::new().with_pin(4, GroupPublic::zero());

            assert!(canned.ensure_key(4, PinEffort::Local).await);
            assert!(!canned.ensure_key(5, PinEffort::Thorough).await);

            assert_eq!(
                canned.efforts(),
                vec![(4, PinEffort::Local), (5, PinEffort::Thorough)],
                "both calls recorded, in order, with the effort each was given"
            );
        });
    }
}

/// Outcome of a per-epoch beacon-key resolve (see [`BeaconResolver`]).
pub enum BeaconResolve {
    /// The epoch's key + share — the mint at the chain's `dkgQual` key epoch
    /// (`beacon::carry::select_carry_scheme`), an exact-epoch ceremony or a
    /// carried one no re-mint superseded.
    Key(BeaconKey),
    /// No usable local material for the epoch: nothing stored at or below it,
    /// a chain-declined or superseded mint, or an undecided (unreadable)
    /// `dkgQual` bit. ⇒ a fallback (pure-multisig) epoch / share-gate demote;
    /// re-resolved on the next edge.
    Absent,
}

/// Resolves the per-epoch [`BeaconKey`] (live-DKG store + `dkgQual`-bit-gated
/// carry-forward). Built at the launch site over the `CeremonyStore`; see
/// `dpos.rs::beacon_share_resolver`. This is the LOCAL DKG material (full
/// polynomial + this node's share) — required to SIGN seed partials and
/// verify individual partials. The polynomial is NOT on-chain, so this stays
/// node-local.
pub type BeaconResolver = Arc<dyn Fn(u64) -> BeaconResolve + Send + Sync>;

/// What W1 does with the key it resolved for an epoch it is about to sign.
///
/// A free fn so the suppression rule is testable without an `Actor`, and so the
/// three outcomes are named rather than implied by a nested `if`.
// A `GroupPublic` (G2) is ~288 B; this is a transient return value matched
// immediately by its one caller and never stored, so the stack copy is cheaper
// than the heap allocation boxing would add — the identical trade `BoundaryOutcome`
// makes (`beacon::keys`).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq, Eq)]
enum OwnKeyPublication {
    /// No quorum-agreed key is recorded for the epoch: publish, exactly as W1
    /// always has. This is every stable (carry-forward) epoch, where no agreement
    /// instance runs at all — and it is why the suppression is conditional: an
    /// unconditional one would stop answering ladder rung 1 for those, and
    /// `repair_keyless_schemes` would never resolve them, leaving their oracles
    /// on `NoKey` and their certs on vote-only admission.
    Publish,
    /// A quorum already published this exact key. The store entry answers rung 1
    /// at a strictly higher provenance, so W1 has nothing to add.
    DeferAgreeing,
    /// A quorum published a DIFFERENT key for this epoch. The agreed one holds
    /// (the tiering in `BeaconKeys::insert` would have kept it anyway); the
    /// divergence is the thing worth saying out loud.
    DeferDiverging(GroupPublic),
}

/// The promote VALUE gate, the share self-probe, and W1 — one unit, in this
/// order, over ONE resolved sample of the beacon material.
///
/// Free fn over its inputs so the three gates and the publish they guard cannot
/// drift apart: W1 must never run for material a gate rejected, and the ordering
/// is what makes that structural rather than a convention.
fn promote_gates(
    group_keys: &BeaconKeys,
    metrics: &BeaconMetrics,
    beacon: Option<&BeaconKey>,
    epoch: Epoch,
) -> Result<(), WithheldReason> {
    // Promote-gate VALUE check (defense-in-depth; f297cc36 extended
    // from key PRESENCE to key VALUE): when a `committee[E]` quorum
    // has already certified a `PK_epoch` for E, a resolver key that
    // DIFFERS is a diverged local reconstruction, whatever produced
    // it. Never sign, W1-publish, or witness-check under it: demote
    // to verify-only; the recompute-heal stores the correct
    // exact-epoch `(PK_E, share)` and its `share_notify` edge
    // re-runs this reconcile, which then promotes with the matching
    // key.
    //
    // The comparand is the agreement artifact, reached through the
    // store (`attested`). It used to be a second, weaker source
    // underneath: the boundary block's own `beacon_outcome`, read
    // by height. That source is strictly worse and no longer
    // exists — it required a block of E to have been produced and
    // stored, so it was blind at exactly the moment the gate is
    // asked (the epoch's first spawn), whereas the artifact is
    // certified BEFORE the epoch starts and covers a stable
    // carry-forward epoch, which no boundary block ever did.
    // ONE READ of the quorum-attested key, used by BOTH the value gate below and
    // the W1 publish further down. It used to be read twice, with a BLS pairing
    // check in between — and the agreement write-back writes that same shared map
    // from its own task on a multi-threaded runtime, so the two reads could
    // legitimately disagree. When they did, the gate had passed on the stale one
    // and W1 arrived holding proof it was wrong, warned, and returned `Ok`: the
    // node signed with material it had just been shown was divergent.
    //
    // Sampling once is the fix, and it is the same rule the rest of this surface
    // already follows — the gates run over ONE sample and the scheme is built from
    // the SAME one. Reacting to the second read instead would only have narrowed
    // the window by the width of a pairing check, while leaving the decision
    // dependent on the atomicity of two reads.
    let attested = group_keys.attested(epoch.get());
    if let Some((sharing, _, _)) = beacon {
        let pk = *sharing.public();
        if let Some(net) = attested {
            if net != pk {
                metrics.engine_demoted_key_divergence.inc();
                warn!(
                    ?epoch,
                    resolved = %pk_prefix(&pk),
                    network = %pk_prefix(&net),
                    "resolved PK_epoch DIVERGES from the quorum-attested \
                     key — verify-only (promote value-gate)"
                );
                return Err(WithheldReason::KeyDivergence);
            }
        }
    }

    // Promote-gate SHARE check. `CombinedScheme::new` asserts only that
    // the share's INDEX equals this node's participant index — never
    // that its VALUE lies on the sharing. While blocks flow, a bad share
    // is exposed on the notarize path; in a sustained stall there are no
    // proposals, so it is not, and since every Nullify now carries a seed
    // partial and `t == quorum`, one such member on the plane makes the
    // nullify quorum unreachable exactly when nullification is the escape
    // hatch. The probe is purely local (share vs its own sharing), so it
    // also covers the cold-ceremony window where the VALUE gate above is
    // a no-op for want of a network-attested key.
    if let Some((sharing, Some(share), namespace)) = beacon {
        let probe = Round::new(epoch, View::new(1));
        let partial = beacon_bls::sign_seed_partial(share, namespace, probe);
        if !beacon_bls::verify_seed_partial(sharing, namespace, probe, &partial) {
            metrics.engine_demoted_bad_share.inc();
            warn!(
                ?epoch,
                "resolved DKG share does not verify against its own sharing — \
                 verify-only (promote share-gate)"
            );
            return Err(WithheldReason::BadShare);
        }
    }

    // W1 (P1) — publish `PK_epoch` into the cross-epoch group-key
    // map BEFORE the engine exists (never inside `spawn_engine`):
    // the engine cannot cast a vote before it is spawned, so every
    // node that votes on epoch E finds `group_keys[E]` populated at
    // its first vote — the same-epoch quorum argument (`b = 0`).
    // Infallible here: `beacon` is already resolved and the
    // share-gate above demoted the shareless case to verify-only.
    // One epoch later this same entry IS the boundary warm (W2):
    // `E+1`'s gate needs `PK_E` and reads the map, no I/O.
    //
    // It stands down where the epoch-key agreement plane already
    // spoke: an artifact is a `committee[epoch]` quorum over this
    // exact key, and a local reconstruction has nothing to add to
    // it. Rung 1 of the ladder is answered by the artifact's entry
    // either way, which is what `repair_keyless_schemes` depends on
    // (the sweep consults no local DKG material BECAUSE W1 fills the
    // store).
    if let Some((sharing, _, _)) = beacon {
        let pk = *sharing.public();
        match own_key_publication(attested, pk) {
            OwnKeyPublication::Publish => {
                // The value fingerprint is the point: a restarted signer whose
                // carried-forward key diverged from the network's is only
                // diagnosable by grepping this line across nodes (soak
                // 2026-07-14 v5@epoch77 reject{bad_signature}).
                info!(
                    ?epoch,
                    group_public = %pk_prefix(&pk),
                    "W1: publishing own epoch group key"
                );
                group_keys.set_pk(epoch.get(), pk, KeySource::LocalDkg);
            }
            OwnKeyPublication::DeferAgreeing => debug!(
                ?epoch,
                group_public = %pk_prefix(&pk),
                "W1: the agreement plane already published this epoch's key"
            ),
            // The divergence witness the suppressed `set_pk` used to
            // raise from inside the store. Same metric name, so a
            // dashboard watching for a split key still sees this one.
            // UNREACHABLE FROM HERE, and that is the point of the single read
            // above: this arm needs `attested` to be `Some(other)`, which the
            // value gate has already turned into a demote. It stays because
            // `own_key_publication` is a pure function with its own tests and its
            // own contract — a caller that samples differently could still reach
            // it — but from `promote_gates` it cannot fire, and if it ever does,
            // the single-read property has been broken.
            OwnKeyPublication::DeferDiverging(agreed) => {
                metrics::counter!(
                    "dpos_group_key_conflict_total",
                    "winner" => "agreed_kept"
                )
                .increment(1);
                warn!(
                    ?epoch,
                    resolved = %pk_prefix(&pk),
                    agreed = %pk_prefix(&agreed),
                    "resolved PK_epoch DIVERGES from the quorum-agreed key; \
                     keeping the agreed one"
                );
            }
        }
    }

    Ok(())
}

/// W3 (P1) — best-effort group-key backfill for the PREVIOUS epoch, off
/// the vote path: covers a node promoted mid-`E` that never ran `E−1`'s
/// engine (no W1 entry for `E−1`). Insert ONLY on success — a failure is
/// never cached, so this re-attempts on every reconcile edge (boundary /
/// share / spawn_unblocked / vote_backup). A warm-up, not the boundary
/// repair path: the first block of `E+1` is verified ~1 s after the
/// spawn, so the repair that fires there is the per-vote lazy resolve.
///
/// Runs for BOTH roles, before the role match — it is not part of the promote
/// decision and must not be folded into one.
fn w3_backfill(group_keys: &BeaconKeys, resolver: &BeaconResolver, epoch: Epoch) {
    let Some(prev) = epoch.get().checked_sub(1) else {
        return;
    };
    if group_keys.cached_only(prev).is_some() {
        return;
    }
    // Best-effort: an undecided resolve is NOT backfilled — the
    // next edge re-attempts.
    if let BeaconResolve::Key((sharing, _, _)) = resolver(prev) {
        let pk = *sharing.public();
        debug!(
            epoch = prev,
            group_public = %pk_prefix(&pk),
            "W3: backfilling previous-epoch group key from own DKG material"
        );
        group_keys.set_pk(prev, pk, KeySource::LocalDkg);
    }
}

fn own_key_publication(agreed: Option<GroupPublic>, resolved: GroupPublic) -> OwnKeyPublication {
    match agreed {
        None => OwnKeyPublication::Publish,
        Some(agreed) if agreed == resolved => OwnKeyPublication::DeferAgreeing,
        Some(agreed) => OwnKeyPublication::DeferDiverging(agreed),
    }
}

/// The live provider: today's handles behind the trait.
///
/// Deliberately a THIN adapter — every method delegates to the function or store
/// that already implements it. Nothing is reimplemented here, so this phase
/// cannot change behaviour; the bodies relocate in later phases, once this is
/// their only caller.
pub(crate) struct PlaneRandomness {
    seeds: super::certify::SeedStore,
    keys: BeaconKeys,
    verify: Option<BeaconVerify>,
    resolver: BeaconResolver,
    /// The oracle's two inputs, held HERE and nowhere above: a scheme reads the
    /// live ceremony store through the oracle rather than being handed a copy of
    /// the material, so a ceremony that finishes after the scheme was built is
    /// picked up on the next vote instead of at the next epoch.
    ceremony: CeremonyStore,
    dkg_qual: DkgQualFor,
    held: Option<super::keys::AgreedKeys>,
    pull: Option<super::keys::AgreedKeys>,
    /// The by-round σ pull. `None` on a node with no resolver seam, where a
    /// witness miss stays what it is today: a skipped view.
    pull_seed: Option<super::seed_resolver::PullSeed>,
    participation: Arc<Notify>,
    metrics: BeaconMetrics,
    chain_id: u64,
}

/// Everything [`PlaneRandomness::build`] needs, in one value.
///
/// One field per handle the provider holds, named after it. A parameter object
/// rather than nine positions: the four `Option`/`Arc` slots in the middle are
/// type-compatible with each other, so a transposed pair compiles and only shows
/// up as a provider that silently answers from the wrong rung.
pub(crate) struct PlaneRandomnessConfig {
    pub(crate) seeds: super::certify::SeedStore,
    pub(crate) keys: BeaconKeys,
    pub(crate) verify: Option<BeaconVerify>,
    pub(crate) resolver: BeaconResolver,
    pub(crate) ceremony: CeremonyStore,
    pub(crate) dkg_qual: DkgQualFor,
    pub(crate) held: Option<super::keys::AgreedKeys>,
    pub(crate) pull: Option<super::keys::AgreedKeys>,
    pub(crate) pull_seed: Option<super::seed_resolver::PullSeed>,
    pub(crate) participation: Arc<Notify>,
    pub(crate) metrics: BeaconMetrics,
    pub(crate) chain_id: u64,
}

impl PlaneRandomness {
    pub(crate) fn build(cfg: PlaneRandomnessConfig) -> Arc<dyn Randomness> {
        let PlaneRandomnessConfig {
            seeds,
            keys,
            verify,
            resolver,
            ceremony,
            dkg_qual,
            held,
            pull,
            pull_seed,
            participation,
            metrics,
            chain_id,
        } = cfg;
        Arc::new(Self {
            seeds,
            keys,
            verify,
            resolver,
            ceremony,
            dkg_qual,
            held,
            pull,
            pull_seed,
            participation,
            metrics,
            chain_id,
        })
    }

    /// One resolve of this node's local DKG material for `epoch`.
    ///
    /// PRIVATE, and it stays private: the share reached the scheme through this
    /// until the oracle existed. What is left of it is the promote GATES, which
    /// need the polynomial and the share to judge this node's own fitness — a
    /// decision that never leaves the plane.
    fn material(&self, epoch: u64) -> Option<BeaconKey> {
        match (self.resolver)(epoch) {
            BeaconResolve::Key(key) => Some(key),
            BeaconResolve::Absent => None,
        }
    }

    /// The oracle for `epoch`, bound to this node's consensus seat when it has
    /// one. `me = None` is the verifier flavour: it never signs a partial.
    fn oracle_at(&self, epoch: u64, me: Option<Participant>) -> Arc<dyn SeedOracle> {
        Arc::new(BeaconOracle {
            epoch,
            ceremony: self.ceremony.clone(),
            keys: self.keys.clone(),
            dkg_qual: self.dkg_qual.clone(),
            namespace: beacon_bls::seed_namespace(&fluentbase_bls::fluent_namespace(self.chain_id)),
            me,
            minted_at: Arc::new(Mutex::new(None)),
            warned_threshold_mismatch: Arc::new(AtomicBool::new(false)),
            warned_seat_mismatch: Arc::new(AtomicBool::new(false)),
            metrics: self.metrics.clone(),
        })
    }
}

impl Randomness for PlaneRandomness {
    fn record_seed(&self, verified: VerifiedSeed) {
        self.seeds.record(verified);
    }

    fn quarantine_seed(&self, round: Round, seed: BlsSignature) {
        self.seeds.quarantine(round, seed);
    }

    fn on_invalid_seed(&self, epoch: u64) -> InvalidSeed {
        self.keys.on_invalid_seed(epoch)
    }

    fn fetch_seed(&self, round: Round) -> BoxFuture<'_, bool> {
        match self.pull_seed.as_ref() {
            Some(pull) => pull(round),
            None => Box::pin(std::future::ready(false)),
        }
    }

    fn seed_for(&self, round: Round) -> Option<Seed> {
        self.seeds.lookup(round).map(|signature| Seed {
            target_round: round,
            signature,
        })
    }

    fn seed_edge(&self) -> Arc<Notify> {
        self.seeds.notifier()
    }

    fn check_witness(&self, parent_epoch: u64, seed: &Seed) -> WitnessCheck {
        super::resolve::resolve_witness(&self.keys, self.verify.as_ref(), parent_epoch, seed)
    }

    fn share_probe(&self, epoch: Epoch) -> ShareProbe {
        // The CHEAP half of the promote decision: both checks below are store
        // reads, so this can sit ahead of the caller's boundary-block lookup and
        // spare a member that cannot participate a marshal read on every
        // participation edge.
        let material = self.material(epoch.get());
        if self.mandatory_at(epoch.get()) && material.is_none() {
            self.metrics.engine_demoted_no_polynomial.inc();
            return ShareProbe::Withheld(WithheldReason::NoUsableShare);
        }
        // THE VALUE CHECK BELONGS HERE TOO, and not only in `signer_scheme`.
        // `signer_scheme` runs once, at the spawn; this probe runs on EVERY
        // reconcile edge, including while an engine is already live. A quorum can
        // attest a key AFTER this node spawned — the agreement write-back lands
        // whenever it lands — and until this check existed nothing revisited that
        // decision: `reconcile_roles` returns early while the handle is alive,
        // before every gate, so a node that learned its key was divergent went on
        // voting with it until a halt or until the epoch fell below the frontier.
        //
        // Safe to act on, because the verdict is STABLE rather than transient: the
        // comparand is the attested tier, which only ever appears once and is never
        // downgraded. A `Withheld` here will not flap back to `Ready` next tick.
        if let Some((sharing, _, _)) = material.as_ref() {
            let pk = *sharing.public();
            if self.keys.attested(epoch.get()).is_some_and(|net| net != pk) {
                self.metrics.engine_demoted_key_divergence.inc();
                return ShareProbe::Withheld(WithheldReason::KeyDivergence);
            }
        }
        ShareProbe::Ready
    }

    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        let material = self.material(epoch.get());
        // THE SHARE GATE, AGAIN — and it has to be here, not only in
        // `share_probe`. The two run over DIFFERENT samples: the probe answers
        // before the caller's boundary-block lookup, this runs after it, and the
        // material can vanish across that await (an agreement artifact landing
        // makes the carry-divergence guard refuse it; the ceremony-store prune
        // can drop the mint).
        //
        // Every guard below is written `if let Some(material)`, so on `None` all
        // three pass VACUOUSLY — and `build_signer` with no beacon material
        // happily returns a working PURE-MULTISIG scheme. Without this check the
        // verdict for a beacon-active epoch would be `Signs(seedless)`: the node
        // spawns a participating engine, every vote it casts carries no seed
        // partial, peers reject the whole vote, and it silently subtracts itself
        // from the quorum while believing it is signing. That is exactly the
        // wedge the share gate exists to prevent, re-entered through the back
        // door — and no `engine_demoted_*` counter would move.
        //
        // Regression note: HEAD took ONE sample before the await and fed it to
        // both the gate and the scheme, so this could not arise. Splitting the
        // operation is what created the second sample; the split's own comment
        // reasoned about the Absent→Key direction only.
        if self.mandatory_at(epoch.get()) && material.is_none() {
            self.metrics.engine_demoted_no_polynomial.inc();
            return SignerVerdict::Withheld(WithheldReason::NoUsableShare);
        }
        // The gates run over ONE sample, and the scheme is built from the SAME
        // one — that coherence is why the two are not separate operations.
        if let Err(reason) = promote_gates(&self.keys, &self.metrics, material.as_ref(), epoch) {
            return SignerVerdict::Withheld(reason);
        }
        // The W1 ordering tripwire that used to sit here is GONE with the copy it
        // guarded: publish-before-handover was a data dependency only because the
        // scheme took a snapshot of the key. An oracle reads the store live, so a
        // key that lands after the scheme is built is picked up on the next vote
        // and the ordering it asserted cannot be violated.
        let committee = match crate::scheme::epoch_committee_from_snapshot(snap) {
            Ok(c) => c,
            Err(e) => return SignerVerdict::InvalidCommittee(e),
        };
        let namespace = fluentbase_bls::fluent_namespace(self.chain_id);
        // The seat has to be known BEFORE the oracle is built, because the oracle
        // checks the share's index against it — taking it from the share would
        // make that check compare a value against itself. A probe scheme is how
        // this node asks the committee where it sits.
        let seat = fluentbase_bls::scheme::build_signer(
            &namespace,
            committee.bimap.clone(),
            keypair,
            epoch.get(),
            None,
        )
        .and_then(|probe| {
            use commonware_cryptography::certificate::Scheme as _;
            probe.me()
        });
        let Some(seat) = seat else {
            // Not in the committee's map at all — a misconfiguration safety net
            // whose engine is destroyed at the next reconcile.
            //
            // IT STILL CARRIES THE EPOCH'S ORACLE, and the earlier decision to
            // hand it `None` "for exact parity with today's `beacon: None`" was
            // wrong. Parity holds on the ATTESTATION arm — no share, no partial,
            // either way — and breaks on the CERTIFICATE arm: under the pin, this
            // scheme was REPAIRABLE (`apply_pin` could attach `PK_epoch` to an
            // already-registered scheme later), and nothing repairs a scheme any
            // more. `oracle_for` reproduces the old POST-pin state exactly:
            // verify-only because there is no share, but still checking the seed
            // slot of every certificate of the epoch.
            //
            // This is a PRODUCER obligation and cannot be delegated to
            // `EpochSchemeProvider::register`. That guard only runs on an
            // occupied slot, and on a clean live-epoch path — `share_probe`
            // `Ready`, boundary block present, so neither early exit soft-enters
            // — this verdict's scheme is the epoch's FIRST registration. A
            // vacant-slot insert is unconditional, nothing upgrades it
            // afterwards, and the epoch would admit every cleared-seed
            // certificate for the whole retention window while the repair sweep
            // reported it as upgraded.
            return SignerVerdict::RotatedKey(fluentbase_bls::scheme::build_verifier(
                &namespace,
                committee.bimap,
                epoch.get(),
                self.oracle_for(epoch.get()),
            ));
        };
        // `material.is_some()` is what "this node holds a share for the epoch"
        // means, and it is the same sample the gates above just ran over.
        //
        // `oracle_at`, NOT `oracle_for` — the one place on this type that goes
        // round the door `oracle_for` exists to be. It has to, because the seat
        // must ride the oracle here and `oracle_for` builds the `me: None`
        // flavour. So the pre-beacon refusal is INHERITED rather than applied:
        // below `DETERMINISTIC_BOOTSTRAP_EPOCH` the resolver answers `Absent`
        // (`beacon/carry.rs::chain_key_epoch_memoised` returns `Some(None)`
        // there → `NoUsableMint`), `material` is `None`, and `then` never fires.
        // If that ever stops holding, this line starts attaching an oracle to a
        // pre-beacon epoch and every LEGAL seedless certificate of it is refused
        // — so the two facts have to move together. The invariant is already
        // covered, if only as a premise: the pre-beacon half of
        // `a_beacon_active_epoch_with_no_material_is_withheld_and_never_signs_seedless`
        // asserts `Signs(_)` here at `DETERMINISTIC_BOOTSTRAP_EPOCH - 1` under an
        // `Absent` resolver.
        let oracle = material
            .is_some()
            .then(|| self.oracle_at(epoch.get(), Some(seat)));
        match fluentbase_bls::scheme::build_signer(
            &namespace,
            committee.bimap,
            keypair,
            epoch.get(),
            oracle,
        ) {
            Some(scheme) => SignerVerdict::Signs(scheme),
            None => unreachable!("the seat probe above already proved membership"),
        }
    }

    fn participation_edge(&self) -> Arc<Notify> {
        self.participation.clone()
    }

    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        // THE BEACON-ACTIVE RULE, enforced HERE so there is one door instead of
        // three. An oracle means "this epoch is beacon-active" to
        // `verify_certificate`, which then rejects every seedless certificate
        // under it — so an oracle on a pre-beacon epoch rejects every LEGAL
        // certificate there. The soft-enter path and the cert-inlet both take
        // their oracle from here, so refusing at the SOURCE makes the invariant
        // hold by construction for every caller.
        self.mandatory_at(epoch)
            .then(|| self.oracle_at(epoch, None))
    }

    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            // Nothing to acquire below the bootstrap epoch: no key was ever
            // minted there, and `oracle_for` refuses to attach one anyway.
            if !self.mandatory_at(epoch) {
                return false;
            }
            let sources = super::keys::KeySources {
                held: self.held.as_ref(),
                // The ONLY thing the effort decides: may this call spend a peer
                // round-trip. `Local` callers (cert ingress, soft-enter) run
                // against a ~1 s verify budget and the network rung's is seconds.
                pull: match effort {
                    PinEffort::Local => None,
                    PinEffort::Thorough => self.pull.as_ref(),
                },
                // THE PROVENANCE FLOOR APPLIES TO BOTH EFFORTS.
                //
                // A wrong key here is terminal for the epoch: everything reading
                // this store treats it as the network's `PK_epoch`, and a wrong
                // one rejects every legal certificate of that epoch. A MISSING key
                // only degrades the epoch to vote-only admission, which still
                // verifies the multisig quorum, committee membership and subject
                // binding. This node's own W1/W3 reconstruction is the one tier
                // that can diverge from the network's key (soak 2026-07-14), so no
                // effort may write it here — the trade only ever runs one way.
                //
                // THIS COST HALF THE BLOCK RATE ON ITS FIRST ATTEMPT, and the
                // reason is worth keeping. With the floor on, a self-reconstructed
                // store entry stops answering, so the call falls through to the
                // `held` rung — which opens with `chain_key_epoch`, a reverse walk
                // over `(BOOTSTRAP, E]`. On a stable committee the answer sits at
                // the bootstrap epoch, so that walk ran its full length on EVERY
                // certificate: 26-27 devnet blocks per 60 s against a target of
                // 60, reproduced on an idle machine and bisected to this line.
                //
                // What made it affordable was not weakening the floor but
                // memoising the walk (`chain_key_epoch_memoised`): a successful
                // answer is a function of frozen bits alone and therefore eternal,
                // so caching it adds no trust, and monotonicity makes a miss cost
                // one step instead of `E - BOOTSTRAP`. Capping the walk is
                // NOT an alternative — on a stable committee the right answer is
                // arbitrarily deep, which is why `CARRY_WALK_CAP` was retired
                // without replacement.
                store_floor: Some(super::keys::KeySource::Carried),
            };
            self.keys.get_pk(epoch, sources).await.is_some()
        })
    }

    fn key_edge(&self) -> Arc<Notify> {
        self.keys.notifier()
    }

    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch) {
        w3_backfill(&self.keys, &self.resolver, reconciled);
        let oldest = entered_frontier
            .get()
            .saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
        self.keys.retain_from(oldest);
        // The quarantine rides the SAME window as the key store, because it is
        // waiting on exactly what that store retains: past the retention edge no
        // key can arrive any more, so a held σ can never be promoted and is only
        // memory a peer could grow.
        self.seeds.retain_quarantine_from(oldest);
    }

    fn observe_cert(&self, epoch: u64) {
        let oldest = epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
        self.keys.retain_from(oldest);
        self.seeds.retain_quarantine_from(oldest);
    }
}
