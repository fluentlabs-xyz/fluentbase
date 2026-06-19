//! Stage-2 beacon seed-verify at the consensus *certify* hook.
//!
//! `CertifiableAutomaton::certify` is the phase BETWEEN notarization and
//! finalization (commonware `consensus/src/lib.rs`): simplex only constructs a
//! `Finalize` for a round whose proposal `is_certified()`, and a `certify` that
//! resolves `false` triggers `FailedCertification` → Nullify → next leader
//! (liveness-preserving — it never halts the chain).
//!
//! ## Why a certify-stage gate is needed (standalone soundness)
//!
//! The per-epoch beacon key `PK_E` for a CHANGE-epoch is asserted by the first
//! block's proposer in `OrderBlock.beacon_outcome` and gated PRE-vote by the "C"
//! share-on-polynomial check ([`crate::application::beacon_gate_decision`]). That
//! gate alone is NOT standalone-sound: a Byzantine proposer can forge a `PK_E`
//! whose polynomial matches ≤ quorum−1 honest shares, slip past the per-node C
//! check on the nodes whose shares it captured, and finalize a forged key —
//! which then poisons `getEpochBeaconKey(E)` for the rest of the epoch.
//!
//! [`CombinedScheme::verify_certificate`](fluentbase_bls) already verifies the
//! recovered seed against each node's OWN resolved `PK_E` (the real local DKG /
//! on-chain key), so a *non-boundary* block's seed is already pinned to the real
//! key at notarization-accept. What it does NOT cover is the boundary block: the
//! honest recovered seed legitimately verifies against the REAL key (held
//! locally), but the block ASSERTS a possibly-different `beacon_outcome` key that
//! becomes authoritative on-chain. This gate closes exactly that gap: on a
//! boundary block (`beacon_outcome = Some`), verify the recovered seed against the
//! block's OWN asserted `PK_E`. A forged assertion ≠ the real key ⇒ the real seed
//! does not verify against it ⇒ `certify = false` ⇒ Nullify before the forged key
//! finalizes.
//!
//! ## Seed reachability at certify
//!
//! `certify(round, payload)` is handed only the round and the block digest — NOT
//! the recovered seed. The seed lives in the round's notarization certificate
//! (`Activity::Notarization.certificate.seed()`). The simplex voter reports the
//! notarization (`reporter.report(Activity::Notarization)`, fired from `notify`'s
//! `try_broadcast_notarization`) BEFORE the next loop iteration scans
//! `certify_candidates()` and calls `certify` — for BOTH self-assembled and
//! peer-received notarizations (`broadcast_notarization` returns the cert once
//! `add_notarization` has set it, regardless of origin). So a [`Reporter`] that
//! records `round → recovered seed` into a shared [`SeedStore`] makes the seed
//! reachable at certify. [`crate::spec_exec::Mailbox`] (which already extracts the
//! seed from each notarization) is that writer.
//!
//! ## Determinism
//!
//! commonware requires `certify` to be deterministic across honest nodes. Both
//! inputs are agreed: the seed is unique by threshold construction (any ≥t
//! partials recover the same value), and the asserted `PK_E` is part of the
//! agreed block body (`beacon_outcome`). Non-boundary blocks are left to
//! `verify_certificate` (re-checking them here against a per-node on-chain read
//! would be both redundant AND a liveness hazard — a node lagging on deferred
//! execution reads an uncommitted `PK_E` while a caught-up node reads it, so the
//! two would disagree).

use crate::{
    application::{ExecutedChain, FluentApp, OrderingAssembler},
    beacon::{
        outcome::{group_public_key, parse_outcome},
        seed::GroupPublic,
    },
    epocher::OriginEpocher,
    order_block::OrderBlock,
};
use commonware_consensus::{
    marshal::{
        core::Mailbox as MarshalMailbox,
        standard::{Inline, Standard},
    },
    types::{Epoch, Round},
    Automaton, CertifiableAutomaton, Relay,
};
use commonware_runtime::{Clock, Metrics, Spawner};
use commonware_utils::channel::{fallible::OneshotExt as _, oneshot};
use fluentbase_bls::{beacon::verify_seed, BlsSignature, Scheme as BlsScheme};
use rand_08::Rng;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tracing::{info, warn};

/// Bound on retained `round → seed` entries. The certify gate reads the seed for
/// a round in the iteration immediately after the round notarized, so only a tiny
/// trailing window is ever live. Generous slack: a seed is 48 B, so a few thousand
/// entries is negligible memory, but a round that notarizes while this node lags a
/// long single block at a boundary may stay in the active certify window for many
/// notarizations — evicting it would drop a still-certifiable round and force a
/// `false` verdict (an unnecessary Nullify). Size the window well past any
/// realistic in-flight certify backlog so an active round is never evicted.
const SEED_RETENTION: usize = 4096;

/// Shared, bounded `round → recovered seed` map. Written by the notarization
/// [`Reporter`](commonware_consensus::Reporter) ([`crate::spec_exec::Mailbox`]),
/// read by [`BeaconCertify::certify`].
pub type SeedStore = Arc<Mutex<BTreeMap<Round, BlsSignature>>>;

/// Construct an empty [`SeedStore`].
pub fn new_seed_store() -> SeedStore {
    Arc::new(Mutex::new(BTreeMap::new()))
}

/// Record the recovered seed for `round`, evicting the oldest entries past
/// [`SEED_RETENTION`]. Idempotent: the seed is unique per round, so a re-report
/// (peer cert after self-assembly, or replay) writes the same value.
pub fn record_seed(store: &SeedStore, round: Round, seed: BlsSignature) {
    let Ok(mut map) = store.lock() else {
        // A poisoned lock means a prior panic while holding it — the seed gate
        // can no longer function; log once rather than propagate a panic into
        // the reporter hot path.
        warn!("beacon certify seed store poisoned; dropping recorded seed");
        return;
    };
    map.insert(round, seed);
    while map.len() > SEED_RETENTION {
        // Evict the oldest (lowest-round) entry. `BTreeMap` orders by `Round`, so
        // `pop_first` is the lowest round (matches the `outer.rs` eviction idiom).
        map.pop_first();
    }
}

fn lookup_seed(store: &SeedStore, round: Round) -> Option<BlsSignature> {
    store.lock().ok()?.get(&round).copied()
}

type InlineFor<E, XC, A> = Inline<E, BlsScheme, FluentApp<XC, A>, OrderBlock, OriginEpocher>;

/// `Inline` wrapper that adds the Stage-2 beacon seed-verify to `certify`.
///
/// Delegates `Automaton`/`Relay` verbatim to the inner [`Inline`]; overrides
/// [`CertifiableAutomaton::certify`] to (1) run `Inline`'s availability gate
/// (preserving the "block must be fetchable" behaviour, incl. the marshal
/// subscription), then (2) on a CHANGE-epoch boundary block, verify the round's
/// recovered seed against the block's OWN asserted `PK_E`.
pub struct BeaconCertify<E, XC, A>
where
    E: Rng + Spawner + Metrics + Clock,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    inner: InlineFor<E, XC, A>,
    /// Runtime context used to spawn the certify follow-up task (cloned per
    /// `certify` call). `Inline` keeps its own private context; this is a
    /// sibling clone of the same runtime.
    context: E,
    marshal: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
    seeds: SeedStore,
    /// The beacon seed signing namespace (`fluent_namespace(chain_id) ‖
    /// "_BEACON_SEED"`), identical across honest nodes.
    seed_namespace: Vec<u8>,
}

impl<E, XC, A> Clone for BeaconCertify<E, XC, A>
where
    E: Rng + Spawner + Metrics + Clock,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            context: self.context.clone(),
            marshal: self.marshal.clone(),
            seeds: self.seeds.clone(),
            seed_namespace: self.seed_namespace.clone(),
        }
    }
}

impl<E, XC, A> BeaconCertify<E, XC, A>
where
    E: Rng + Spawner + Metrics + Clock,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    pub fn new(
        inner: InlineFor<E, XC, A>,
        context: E,
        marshal: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
        seeds: SeedStore,
        seed_namespace: Vec<u8>,
    ) -> Self {
        Self {
            inner,
            context,
            marshal,
            seeds,
            seed_namespace,
        }
    }
}

impl<E, XC, A> Automaton for BeaconCertify<E, XC, A>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    type Digest = <InlineFor<E, XC, A> as Automaton>::Digest;
    type Context = <InlineFor<E, XC, A> as Automaton>::Context;

    async fn genesis(&mut self, epoch: Epoch) -> Self::Digest {
        self.inner.genesis(epoch).await
    }

    async fn propose(&mut self, context: Self::Context) -> oneshot::Receiver<Self::Digest> {
        self.inner.propose(context).await
    }

    async fn verify(
        &mut self,
        context: Self::Context,
        payload: Self::Digest,
    ) -> oneshot::Receiver<bool> {
        self.inner.verify(context, payload).await
    }
}

impl<E, XC, A> CertifiableAutomaton for BeaconCertify<E, XC, A>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    async fn certify(&mut self, round: Round, payload: Self::Digest) -> oneshot::Receiver<bool> {
        // First the inner availability gate (its certify resolves `true` only
        // once the block is fetchable, and keeps the request pending until then —
        // exactly today's behaviour). We then layer the seed-verify on top.
        let avail_rx = self.inner.certify(round, payload).await;

        let marshal = self.marshal.clone();
        let seeds = self.seeds.clone();
        let namespace = self.seed_namespace.clone();

        let (tx, rx) = oneshot::channel();
        self.context
            .clone()
            .with_label("beacon_certify")
            .with_attribute("round", round)
            .spawn(move |_| async move {
                let decision =
                    certify_decision(avail_rx, marshal, round, payload, &seeds, &namespace);
                drive_certify(tx, decision).await;
            });

        rx
    }
}

/// Resolve the voter's certify `tx` from a `decision` future, halt-safely.
///
/// CRITICAL halt-safety contract (commonware voter, `actor.rs:949-981`): the voter
/// awaits the matching `rx` and treats `Err` (us dropping `tx` unsent) as a
/// NON-event — it only `debug!`-logs, never times out a NOTARIZED round, and never
/// re-issues `certify` (`round.rs` `try_certify` returns `None` once `Outstanding`).
/// So dropping `tx` without a verdict WEDGES the round → HALT. This driver therefore
/// resolves `rx` in EXACTLY one of two ways:
///   1. it SENDS the explicit `true`/`false` `decision` verdict, or
///   2. it EXITS because the voter already dropped `rx` (`tx.closed()` fired) —
///      safe, the voter moved on (round pruned / finalized). This arm also aborts
///      the task promptly so it cannot leak (it holds a marshal subscription +
///      scheme clones).
///
/// `decision` MUST never resolve on a non-decidable state — it parks instead (see
/// [`certify_decision`]) — so the only escape from such a state is the `tx.closed()`
/// arm, mirroring `Inline`'s own pending semantics (marshal `inline.rs`
/// `await_block_subscription`).
async fn drive_certify(
    mut tx: oneshot::Sender<bool>,
    decision: impl std::future::Future<Output = bool>,
) {
    tokio::select! {
        _ = tx.closed() => {}
        verdict = decision => {
            tx.send_lossy(verdict);
        }
    }
}

/// Resolve the certify verdict for a round, or PARK forever (never resolve) on any
/// terminal/transient inability to decide.
///
/// Returning here means the caller will `send_lossy(verdict)` — a definitive
/// `true`/`false`. Therefore every code path that cannot produce a definitive
/// verdict (inner gate closed, block un-fetchable) MUST instead park via
/// [`std::future::pending`], so the ONLY escape from a non-decidable state is the
/// caller's `tx.closed()` arm (= the voter dropped its receiver and moved on). This
/// is what keeps the voter's `rx` from ever resolving to `Err`, which would wedge
/// the notarized round into a HALT (see [`CertifiableAutomaton::certify`] above).
///
/// `avail_rx` is the inner [`Inline::certify`] verdict. `Inline` only ever
/// `send_lossy(true)` once the block is fetchable, or holds its sender pending /
/// drops it on teardown (it NEVER sends `false` — confirmed in marshal
/// `inline.rs`). So:
/// - `Ok(true)` → block is available, proceed to the seed check.
/// - `Ok(false)` → not a real signal `Inline` can emit; treat defensively as "not
///   yet available → keep pending" (park), NOT "certify false".
/// - `Err(_)` → inner channel closed (teardown / prune) → keep pending (park).
async fn certify_decision(
    avail_rx: oneshot::Receiver<bool>,
    marshal: MarshalMailbox<BlsScheme, Standard<OrderBlock>>,
    round: Round,
    payload: crate::digest::Digest,
    seeds: &SeedStore,
    namespace: &[u8],
) -> bool {
    // Inner availability gate. Any non-`true` outcome is "not yet decidable" → park.
    if !matches!(avail_rx.await, Ok(true)) {
        return std::future::pending().await;
    }

    // The block is available — fetch it to inspect its `beacon_outcome`. The inner
    // gate has just confirmed availability, so this resolves from the marshal
    // buffer. `subscribe_by_digest` registers a subscription that stays pending
    // until the block arrives; it only resolves `Err` if marshal drops the response
    // (terminal teardown). On that `Err` we PARK (do NOT conclude `false` — that
    // would be a non-deterministic Nullify); the caller's `tx.closed()` arm is the
    // only escape.
    let block_rx = marshal.subscribe_by_digest(Some(round), payload).await;
    let Ok(block) = block_rx.await else {
        return std::future::pending().await;
    };

    seed_certify_verdict(&block, round, seeds, namespace)
}

/// The seed-verify verdict for an already-available, already-notarized block.
///
/// - Non-boundary block (`beacon_outcome = None`): the seed was already pinned to
///   the real `PK_E` by `CombinedScheme::verify_certificate` at notarization —
///   nothing to add → `true`.
/// - Boundary block (`beacon_outcome = Some`): the asserted `PK_E` is what gets
///   committed on-chain, so the round's recovered seed MUST verify against it. A
///   forged assertion ⇒ the real seed fails ⇒ `false` → Nullify.
///   - Malformed `beacon_outcome` ⇒ `false` (an honest proposer never emits one).
///   - No recorded seed for the round ⇒ `false`: a boundary block carrying an
///     asserted key MUST have a seeded notarization (the always-active beacon
///     invariant — a seedless quorum cannot form on a beacon-active epoch), so a
///     missing seed here means a beacon-active boundary could not be confirmed.
fn seed_certify_verdict(
    block: &OrderBlock,
    round: Round,
    seeds: &SeedStore,
    namespace: &[u8],
) -> bool {
    let Some(bytes) = block.beacon_outcome.as_ref() else {
        return true; // non-boundary block: covered by verify_certificate
    };
    let outcome = match parse_outcome(bytes) {
        Ok(o) => o,
        Err(e) => {
            warn!(
                height = block.height,
                ?e,
                "beacon certify: beacon_outcome failed to parse — certify false"
            );
            return false;
        }
    };
    let pk_e: &GroupPublic = group_public_key(&outcome);
    let Some(seed) = lookup_seed(seeds, round) else {
        warn!(
            height = block.height,
            ?round,
            "beacon certify: boundary block notarized without a recorded seed — certify false"
        );
        return false;
    };
    let ok = verify_seed(pk_e, namespace, round, &seed);
    if ok {
        info!(
            height = block.height,
            ?round,
            "beacon certify: boundary seed verified vs asserted PK_E"
        );
    } else {
        warn!(
            height = block.height,
            ?round,
            "beacon certify: boundary seed FAILED vs asserted PK_E — certify false (Nullify)"
        );
    }
    ok
}

impl<E, XC, A> Relay for BeaconCertify<E, XC, A>
where
    E: Rng + Spawner + Metrics + Clock + Send + 'static,
    XC: ExecutedChain,
    A: OrderingAssembler,
{
    type Digest = <InlineFor<E, XC, A> as Relay>::Digest;
    type PublicKey = <InlineFor<E, XC, A> as Relay>::PublicKey;
    type Plan = <InlineFor<E, XC, A> as Relay>::Plan;

    async fn broadcast(&mut self, payload: Self::Digest, plan: Self::Plan) {
        self.inner.broadcast(payload, plan).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::outcome::{encode_outcome, DkgOutcome},
        digest::Digest,
    };
    use commonware_consensus::types::{Epoch as TEpoch, View};
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
    use fluentbase_bls::{
        beacon::{recover_seed, seed_namespace, sign_seed_partial},
        fluent_namespace, PeerPubkey,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    fn round_at(view: u64) -> Round {
        Round::new(TEpoch::new(1), View::new(view))
    }

    // The verdict logic is the safety-critical core; it is pure over (block,
    // round, seed store, namespace) so we exercise it directly without standing up
    // a marshal/runtime. We deal a REAL committee DKG (same path the production
    // C-gate validates), so `beacon_outcome` round-trips through
    // `encode_outcome`/`parse_outcome`/`group_public_key` and the recovered seed
    // genuinely verifies against the dealt group key.

    /// Deal a real `n`-party committee DKG with a fixed RNG seed; return the typed
    /// outcome (its group key is the recovered seed's PK) and the per-player shares.
    fn deal_committee(seed: u64, n: u32) -> (DkgOutcome, Vec<Share>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..n).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, share_map) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        let shares: Vec<Share> = share_map.values().to_vec();
        (outcome, shares)
    }

    /// Recover the seed using the outcome's own public sharing.
    fn recover_seed_for(
        outcome: &DkgOutcome,
        shares: &[Share],
        ns: &[u8],
        round: Round,
    ) -> BlsSignature {
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, ns, round))
            .collect();
        recover_seed::<N3f1>(outcome.public(), &partials).expect("recover")
    }

    fn boundary_block(beacon_outcome: Option<Vec<u8>>) -> OrderBlock {
        OrderBlock {
            parent: Digest(alloy_primitives::B256::ZERO),
            height: 100,
            timestamp: 1,
            fee_recipient: Default::default(),
            gas_limit: 30_000_000,
            extra_data: Default::default(),
            result: Default::default(),
            txs: Vec::new(),
            beacon_outcome: beacon_outcome.map(Into::into),
            beacon_seed: None,
        }
    }

    #[test]
    fn non_boundary_block_certifies_without_seed() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let store = new_seed_store();
        let block = boundary_block(None); // beacon_outcome = None ⇒ non-boundary
        assert!(seed_certify_verdict(&block, round_at(7), &store, &ns));
    }

    #[test]
    fn boundary_block_certifies_on_valid_seed() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let r = round_at(7);
        let (outcome, shares) = deal_committee(1, 5);
        let seed = recover_seed_for(&outcome, &shares, &ns, r);
        let store = new_seed_store();
        record_seed(&store, r, seed);

        let block = boundary_block(Some(encode_outcome(&outcome)));
        assert!(
            seed_certify_verdict(&block, r, &store, &ns),
            "honest boundary: recovered seed verifies vs the asserted PK_E"
        );
    }

    #[test]
    fn boundary_block_nullifies_on_forged_pk() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let r = round_at(7);
        // The real committee whose shares produced the cert's seed...
        let (real_outcome, real_shares) = deal_committee(1, 5);
        let seed = recover_seed_for(&real_outcome, &real_shares, &ns, r);
        // ...but the proposer asserts a DIFFERENT (forged) outcome's PK_E.
        let (forged_outcome, _forged_shares) = deal_committee(2, 5);

        let store = new_seed_store();
        record_seed(&store, r, seed);

        let block = boundary_block(Some(encode_outcome(&forged_outcome)));
        assert!(
            !seed_certify_verdict(&block, r, &store, &ns),
            "forged PK_E: the real seed must NOT verify ⇒ certify false ⇒ Nullify"
        );
    }

    /// The PRODUCTION forge path: the byzantine proposer's
    /// `forge_outcome_same_committee` (same helper `build_proposal` calls under
    /// `dpos-devnet-byzantine`) yields a DIFFERENT `PK_E` over the SAME committee;
    /// the round's REAL recovered seed must NOT verify against it ⇒ certify FALSE ⇒
    /// Nullify. The honest outcome over the same recorded seed certifies TRUE. This
    /// is the authoritative proof of the certify-hook Nullify path the byzantine-vrf
    /// smoke relies on (where a colluding byzantine quorum reaches certify).
    #[test]
    fn certify_nullifies_the_production_forge_outcome() {
        use crate::beacon::outcome::forge_outcome_same_committee;
        let ns = seed_namespace(&fluent_namespace(20994));
        let r = round_at(7);
        let (real_outcome, real_shares) = deal_committee(1, 5);
        // The round's seed is the unique threshold signature under the REAL shares.
        let seed = recover_seed_for(&real_outcome, &real_shares, &ns, r);
        let store = new_seed_store();
        record_seed(&store, r, seed);

        // Honest boundary: the real outcome certifies against the recovered seed.
        let honest = boundary_block(Some(encode_outcome(&real_outcome)));
        assert!(
            seed_certify_verdict(&honest, r, &store, &ns),
            "honest boundary: real seed verifies vs the real PK_E"
        );

        // Byzantine boundary: the forged outcome (same committee, different PK_E)
        // does NOT verify against the real recovered seed ⇒ certify false ⇒ Nullify.
        let forged_outcome = forge_outcome_same_committee(&real_outcome);
        assert_ne!(
            group_public_key(&forged_outcome),
            group_public_key(&real_outcome),
            "the production forge must assert a different PK_E"
        );
        let forged = boundary_block(Some(encode_outcome(&forged_outcome)));
        assert!(
            !seed_certify_verdict(&forged, r, &store, &ns),
            "forged PK_E from the production forge path must Nullify at certify"
        );
    }

    #[test]
    fn boundary_block_nullifies_without_recorded_seed() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let r = round_at(7);
        let (outcome, _shares) = deal_committee(1, 5);
        let store = new_seed_store(); // empty — no seed recorded for r
        let block = boundary_block(Some(encode_outcome(&outcome)));
        assert!(
            !seed_certify_verdict(&block, r, &store, &ns),
            "a beacon-active boundary with no recorded seed cannot be confirmed"
        );
    }

    #[test]
    fn malformed_outcome_is_rejected() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let r = round_at(7);
        let store = new_seed_store();
        // A recorded seed is present, but the asserted outcome is undecodable.
        let (outcome, shares) = deal_committee(1, 5);
        record_seed(&store, r, recover_seed_for(&outcome, &shares, &ns, r));
        let block = boundary_block(Some(vec![0xFF; 8]));
        assert!(!seed_certify_verdict(&block, r, &store, &ns));
    }

    // ---- halt-safety driver (`drive_certify`) ----
    //
    // These pin the commonware halt contract: the voter's certify `rx` (actor.rs
    // :949-981) must NEVER resolve to `Err` (us dropping `tx` unsent) while the
    // voter still awaits it — that wedges a notarized round into a HALT. The only
    // permitted resolutions are: send(true), send(false), or task-exit-because-
    // `rx`-already-dropped. We drive the futures with `futures::executor` (no async
    // runtime is wired into this crate's tests) using `now_or_never` to assert
    // (non-)completion without blocking.

    use futures::FutureExt as _;

    /// A decidable decision resolves by SENDING that exact verdict to the voter.
    #[test]
    fn drive_certify_sends_explicit_verdict() {
        for verdict in [true, false] {
            let (tx, rx) = oneshot::channel();
            // Decision resolves immediately ⇒ the driver must complete and send it.
            drive_certify(tx, async move { verdict })
                .now_or_never()
                .expect("driver completes when the decision resolves");
            assert_eq!(
                rx.now_or_never().expect("verdict delivered to voter"),
                Ok(verdict),
                "the driver must forward the exact verdict, never drop `tx`"
            );
        }
    }

    /// Receiver dropped first (voter moved on): the task EXITS cleanly — it does not
    /// hang and it sends nothing. This is the orphan-task-abort / clean-exit path.
    #[test]
    fn drive_certify_exits_cleanly_when_receiver_dropped() {
        let (tx, rx) = oneshot::channel();
        drop(rx); // voter dropped its receiver before we could decide
                  // A never-resolving decision (the parking sentinel) MUST still let the
                  // task finish via the `tx.closed()` arm — otherwise the task leaks.
        drive_certify(tx, std::future::pending::<bool>())
            .now_or_never()
            .expect("driver exits promptly once the receiver is dropped");
    }

    /// A non-decidable (parking) decision with a LIVE receiver must NOT resolve `rx`
    /// at all — no spurious `false`, no dropped `tx`. The round stays pending until
    /// the block becomes decidable or the voter drops `rx`.
    #[test]
    fn drive_certify_parks_without_spurious_verdict() {
        let (tx, mut rx) = oneshot::channel();
        let mut driver = Box::pin(drive_certify(tx, std::future::pending::<bool>()));
        // Poll the driver: it must stay pending (parked), never completing.
        assert!(
            driver.as_mut().now_or_never().is_none(),
            "a parked decision must keep the driver pending (no verdict, no drop)"
        );
        // And the voter's receiver must observe neither a value nor a closed channel
        // (`tx` is still held alive inside the parked driver).
        assert!(
            (&mut rx).now_or_never().is_none(),
            "voter must see the round still pending — not a spurious verdict or Err"
        );
    }

    #[test]
    fn record_seed_is_bounded_and_idempotent() {
        let ns = seed_namespace(&fluent_namespace(20994));
        let (outcome, shares) = deal_committee(1, 5);
        let store = new_seed_store();
        for v in 0..(SEED_RETENTION as u64 + 50) {
            let r = round_at(v);
            record_seed(&store, r, recover_seed_for(&outcome, &shares, &ns, r));
        }
        // Idempotent re-insert (same unique seed) does not grow the map.
        let r0 = round_at(SEED_RETENTION as u64 + 49);
        record_seed(&store, r0, recover_seed_for(&outcome, &shares, &ns, r0));

        let map = store.lock().unwrap();
        assert_eq!(map.len(), SEED_RETENTION, "store is bounded");
        assert!(map.contains_key(&r0), "newest retained");
        assert!(!map.contains_key(&round_at(0)), "oldest evicted");
    }
}
