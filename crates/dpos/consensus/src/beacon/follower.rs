//! The `--cert-follow` follower's randomness: no ceremony, no share, no σ it
//! formed itself — and a real, fillable `PK_epoch` cache, plus a store for the σ
//! its certificates carry.
//!
//! # What this closes
//!
//! A follower used to run [`super::surface::absent`], whose key rungs answer
//! `None` at both efforts for the life of the process. Every certificate it
//! ingested therefore took VOTE-ONLY admission: the attributable `2f+1` multisig
//! quorum was checked, the seed slot was not, so a tampered or cleared seed
//! riding a valid quorum was admitted in silence. Nothing the verification needs
//! was missing — the chain id, an rng and the `committee[epoch]` read are all
//! things a follower already has, and it makes the committee read per
//! certificate. What was missing was a DELIVERY ROUTE for the artifact, which the
//! caller supplies here as [`FollowerRandomnessConfig::fetch`] over the one peer
//! relationship a follower has: its cert upstream.
//!
//! # Why it lives inside the beacon
//!
//! `verify_artifact_for_epoch` and the artifact types are `pub(crate)` and the
//! module is closed (see [`super`]'s docs). Building the provider here keeps them
//! that way: the node hands in capabilities — a fetch closure, a committee source
//! and a `DkgQualFor` — exactly as it already does for the validator plane, and
//! receives back a [`Randomness`] plus one serving read closure.
//!
//! # Trust
//!
//! The upstream is trusted for DELIVERY and for nothing else. A fetched artifact
//! is checked against `committee[epoch]` read from THIS node's own chain state,
//! by the same `verify_artifact_for_epoch` a validator's pull seam uses — so a
//! lying upstream is caught here exactly as a lying peer is caught there. This is
//! the same posture the cert path already takes: the upstream serves the bytes,
//! the local committee decides whether they are true.

use super::{
    artifact::{
        decode_artifact, encode_artifact, verify_artifact_for_epoch, ArtifactError, ArtifactStore,
        CommitteeSource, PULL_MIN_INTERVAL,
    },
    carry::DkgQualFor,
    certify::SeedStore,
    keys::{pk_prefix, AgreedKeyAt, AgreedKeys, BeaconKeys, InvalidSeed, KeySource, KeySources},
    metrics::BeaconMetrics,
    oracle::KeyOnlyOracle,
    outcome::group_public_key,
    plane::ArtifactSource,
    seed::Seed,
    surface::{PinEffort, Randomness, ShareProbe, SignerVerdict, WithheldReason},
    verified_seed::VerifiedSeed,
};
use commonware_consensus::types::{Epoch, Round};
use commonware_runtime::{Clock, Handle, Metrics, Spawner};
use fluentbase_bls::{
    beacon::seed_namespace, fluent_namespace, keys::ValidatorBlsKeypair, oracle::SeedOracle,
    BlsSignature,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use rand_core::OsRng;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    time::SystemTime,
};
use tokio::sync::{mpsc, Notify};
use tracing::{info, warn};

/// Depth of the want channel between [`Randomness::observe_cert`] and the fetch
/// task. Wants are re-issued on every certificate (~1/s) for as long as the epoch
/// stays unresolved, so a full channel costs nothing: the drop is re-asked a
/// second later, and dropping is what keeps `observe_cert` off the network.
const WANT_MAILBOX: usize = 16;

/// One artifact fetch over the follower's cert upstream, by MINTING epoch.
///
/// Returns the wire bytes `decode_artifact` reads, or `None` for every negative
/// alike — no artifact, an upstream too old to know the method, a dead link. The
/// three are one answer here (stay unpinned, ask again) and separating them would
/// only invite a caller to treat one of them as a fault.
pub type ArtifactFetch = Arc<dyn Fn(u64) -> BoxFuture<'static, Option<Vec<u8>>> + Send + Sync>;

/// What [`for_follower`] needs that it cannot build itself. Every field is a
/// capability the node already holds; none of them is beacon state.
pub struct FollowerRandomnessConfig {
    pub chain_id: u64,
    /// `committee[epoch]`, the SOLE authority a fetched artifact is checked
    /// against. Read from this node's own chain state, never from the upstream.
    pub committees: CommitteeSource,
    /// Which epoch minted the key in force at a given epoch (the frozen `dkgQual`
    /// record). A stable epoch runs no agreement and carries its predecessor's
    /// key, so without this the fetch would ask for an artifact that was never
    /// minted.
    pub dkg_qual: DkgQualFor,
    /// The delivery route. See [`ArtifactFetch`].
    pub fetch: ArtifactFetch,
}

/// What the follower's launch site receives back.
pub struct FollowerBeacon {
    /// The consensus-facing surface, for the outer engine and the cert inlet.
    /// ONE instance shared by both: `observe_cert` prunes the store `ensure_key`
    /// reads, and a second provider would make the prune a no-op on a map
    /// nothing else sees.
    pub randomness: Arc<dyn Randomness>,
    /// Serve a held artifact to a TIER-2 follower over
    /// `consensus_getEpochArtifact`, exactly as this node already serves
    /// `getFinalization` out of its bounded cert window.
    pub artifact_bytes: ArtifactSource,
    /// The fetch task. SUPERVISED: it parks rather than returning, so a clean
    /// exit means it died, and a dead fetcher silently returns this node to
    /// vote-only admission for the rest of the process.
    pub fetch_handle: Handle<()>,
}

/// Build the follower's randomness.
///
/// Registers the beacon metric families on `context` — a follower is the sole
/// owner of them on its node class, the same job [`super::surface::absent`] did
/// before it.
pub fn for_follower<E>(context: &E, cfg: FollowerRandomnessConfig) -> FollowerBeacon
where
    E: Clock + Metrics + Spawner + Clone + Send + 'static,
{
    let metrics = BeaconMetrics::default();
    metrics.register(context);

    // RAM-only, and deliberately so: there is no `share_dir` on this path and no
    // durable partition to open. What a restart loses is one fetch per epoch over
    // a link the follower holds open anyway.
    let store = ArtifactStore::new();
    let keys = BeaconKeys::new();

    // The two rungs of the SHIPPED ladder, in the shipped order — a local read of
    // the artifact store, then one delivery over the upstream. Only the second is
    // new; `ensure_key` is handed the first alone, so the certificate path stays
    // network-free by construction rather than by discipline.
    let held = AgreedKeys::new(
        {
            let store = store.clone();
            Arc::new(move |epoch: u64| {
                let key = store.get(epoch).map(|a| *group_public_key(&a.0.group_key));
                Box::pin(async move { key }) as BoxFuture<'static, _>
            })
        },
        cfg.dkg_qual.clone(),
    );
    let pull = AgreedKeys::new(
        fetch_and_verify(
            cfg.chain_id,
            cfg.committees,
            cfg.fetch,
            store.clone(),
            metrics.clone(),
        ),
        cfg.dkg_qual,
    );

    let (want_tx, want_rx) = mpsc::channel(WANT_MAILBOX);
    // Captured before the task exists, as `BeaconKeys::subscribe` requires: a
    // fill landing between the spawn and the task's first poll would be lost to a
    // handle taken inside the loop, and a key arrives ONCE per epoch — unlike a
    // want, nothing re-issues it a second later.
    let key_edge = keys.subscribe();
    let randomness = Arc::new(FollowerRandomness {
        keys: keys.clone(),
        seed_namespace: seed_namespace(&fluent_namespace(cfg.chain_id)),
        held: held.clone(),
        want_tx,
        // RAM-only, like the artifact store above and for the same reason: this
        // path opens no journal partition. What a restart loses is σ the next
        // certificate carries anyway.
        seeds: SeedStore::new(),
        idle: Arc::new(Notify::new()),
        metrics,
    });
    let fetch_handle = {
        // WEAK on purpose: the task holds the receiving end of `want_tx`, so an
        // `Arc` here would keep the sender alive through its own owner and the
        // loop could never tell shutdown from idleness.
        let promoter = Arc::downgrade(&randomness);
        context
            .with_label("follower_artifact_fetch")
            .spawn(move |c| run_fetcher(c, keys, held, pull, want_rx, key_edge, promoter))
    };

    FollowerBeacon {
        randomness,
        artifact_bytes: Arc::new(move |epoch: u64| store.get(epoch).map(|a| encode_artifact(&a))),
        fetch_handle,
    }
}

/// The pull rung's body: one fetch, decoded and CHECKED against
/// `committee[minted_at]` before anything is kept.
///
/// The three failure classes are counted apart because they mean different
/// things about different parties: a miss is the upstream having nothing (or
/// being too old to be asked), an unreadable committee is this node's own chain
/// view lagging, and a rejection is the only one that is an accusation.
fn fetch_and_verify(
    chain_id: u64,
    committees: CommitteeSource,
    fetch: ArtifactFetch,
    store: ArtifactStore,
    metrics: BeaconMetrics,
) -> AgreedKeyAt {
    Arc::new(move |minted_at: u64| {
        let (fetch, committees, store, metrics) = (
            fetch.clone(),
            committees.clone(),
            store.clone(),
            metrics.clone(),
        );
        Box::pin(async move {
            let Some(bytes) = fetch(minted_at).await else {
                metrics.follower_artifact_miss.inc();
                return None;
            };
            let artifact = match decode_artifact(&bytes) {
                Ok(artifact) => artifact,
                Err(e) => {
                    warn!(
                        epoch = minted_at,
                        ?e,
                        "cert-follow: the upstream's epoch artifact does not decode"
                    );
                    metrics.dkg_artifact_rejected.inc();
                    return None;
                }
            };
            match verify_artifact_for_epoch(&mut OsRng, chain_id, &committees, minted_at, &artifact)
            {
                Ok(()) => {}
                // Not the upstream's fault and not a verdict on the artifact:
                // this node's executor has not reached the block that committed
                // `committee[minted_at]`. Drop it and re-ask on the next cert.
                Err(ArtifactError::CommitteeUnreadable(_)) => {
                    metrics.dkg_artifact_unverifiable.inc();
                    return None;
                }
                Err(e) => {
                    warn!(
                        epoch = minted_at,
                        ?e,
                        "cert-follow: REJECTING the upstream's epoch artifact — it does not \
                         carry a committee[epoch] quorum; staying on vote-only admission"
                    );
                    metrics.dkg_artifact_rejected.inc();
                    return None;
                }
            }
            let pk = *group_public_key(&artifact.0.group_key);
            store.insert(minted_at, artifact);
            metrics.follower_artifact_adopted.inc();
            info!(
                epoch = minted_at,
                group_public = %pk_prefix(&pk),
                "cert-follow: PK_epoch obtained and verified against committee[epoch] — \
                 certificates of the epochs it covers leave vote-only admission"
            );
            Some(pk)
        }) as BoxFuture<'static, _>
    })
}

/// The off-path half of the ladder, and the ONLY place a follower touches the
/// network for a key.
///
/// Sequential by construction: one fetch at a time, so a slow upstream costs
/// latency and never a fan-out. The per-epoch throttle is the same
/// [`PULL_MIN_INTERVAL`] budget the plane's pull seam owes its peers — the want
/// arrives on every certificate (~1/s) for as long as the epoch stays unresolved,
/// and without the throttle that would be one upstream round-trip per second per
/// unresolved epoch.
///
/// It carries the QUARANTINE PROMOTE on a second arm. What decides whether a
/// held σ can be served is a key landing, and the key store's own edge is the
/// trigger rather than this task's fetch result: an epoch also resolves through
/// `ensure_key` off an artifact adopted for a DIFFERENT epoch, and a σ waiting on
/// that one would otherwise never be re-checked. The promote rides this task
/// instead of one of its own because the two share a single failure story — a
/// follower that has stopped using `PK_epoch` — and a second task would have to
/// be supervised separately to tell the same thing.
async fn run_fetcher<E: Clock>(
    ctx: E,
    keys: BeaconKeys,
    held: AgreedKeys,
    pull: AgreedKeys,
    mut want_rx: mpsc::Receiver<u64>,
    key_edge: Arc<Notify>,
    promoter: Weak<FollowerRandomness>,
) {
    let mut next_allowed: BTreeMap<u64, SystemTime> = BTreeMap::new();
    loop {
        let epoch = tokio::select! {
            want = want_rx.recv() => match want {
                Some(epoch) => epoch,
                None => break,
            },
            _ = key_edge.notified() => {
                // Gone means the last `Randomness` handle dropped, i.e. shutdown.
                let Some(randomness) = promoter.upgrade() else { break };
                randomness.promote_quarantined();
                continue;
            }
        };
        let now = ctx.current();
        // Keep only the epochs still being held back; otherwise this grows one
        // entry per epoch ever asked for, forever (the same reason
        // `ArtifactPull::throttle` prunes).
        next_allowed.retain(|_, at| *at > now);
        if next_allowed.contains_key(&epoch) {
            continue;
        }
        next_allowed.insert(epoch, now + PULL_MIN_INTERVAL);
        // The full ladder, off-path: the store, then the held artifacts, then the
        // upstream. A hit writes `PK_epoch` into the shared store at
        // `KeySource::Agreed` and memoises the carry for `epoch`, which is what
        // makes the next `ensure_key(Local)` answer without a fetch and stops
        // `observe_cert` re-asking for it.
        let _ = keys
            .get_pk(
                epoch,
                KeySources {
                    held: Some(&held),
                    pull: Some(&pull),
                    store_floor: Some(KeySource::Carried),
                },
            )
            .await;
    }
    // PARK, never return: this handle is supervised, where a clean exit means "a
    // subsystem died, take the node down". The loop ends only when the last
    // `Randomness` handle drops, which is shutdown, and that must not be the
    // thing that cancels the node.
    std::future::pending::<()>().await;
}

/// A follower's [`Randomness`]: negative everywhere a follower genuinely cannot
/// answer, and REAL on the two it can — which key a certificate of an epoch must
/// be checked against, and what σ the certificates it admitted carried.
struct FollowerRandomness {
    keys: BeaconKeys,
    /// The seed-signing domain an assembled σ is verified under. Held because
    /// the oracle needs it and a follower has no plane to ask.
    seed_namespace: Vec<u8>,
    /// The LOCAL rung only. The upstream rung is deliberately unreachable from
    /// [`Randomness::ensure_key`]: ingress resolves per certificate at
    /// [`PinEffort::Local`], which is contractually network-free.
    held: AgreedKeys,
    want_tx: mpsc::Sender<u64>,
    /// σ this node received on a certificate, split the same way the plane splits
    /// it: checked values in the served map, unchecked ones in the quarantine.
    /// RAM-only — see where it is built.
    seeds: SeedStore,
    idle: Arc<Notify>,
    /// Handed to every [`KeyOnlyOracle`] this provider builds, so the keyless
    /// window is counted on the node class where it is most ordinary.
    metrics: BeaconMetrics,
}

impl FollowerRandomness {
    /// Drop everything below `oldest`. The three maps take ONE window because
    /// they wait on one thing: past the retention edge no key can arrive any
    /// more, so a quarantined σ can never be promoted, and a terminal pin is
    /// asked for by the NEXT epoch alone.
    fn retain_from(&self, oldest: u64) {
        self.keys.retain_from(oldest);
        self.seeds.retain_quarantine_from(oldest);
        self.seeds.retain_terminal_from(oldest);
    }

    /// Re-check every held σ, now that a key has landed.
    ///
    /// Load-bearing rather than a completeness item, because on this node class
    /// the keyless window is the ORDINARY state: a follower obtains `PK_epoch`
    /// only by fetching the epoch's artifact, so σ arriving before the key is the
    /// common case and the quarantine is where most of it first lands.
    fn promote_quarantined(&self) {
        for epoch in self.seeds.quarantined_epochs() {
            let Some(oracle) = self.oracle_for(epoch) else {
                continue;
            };
            let (promoted, refused) = self.seeds.promote_epoch(epoch, oracle.as_ref());
            if promoted > 0 || refused > 0 {
                info!(
                    epoch,
                    promoted, refused, "cert-follow: re-checked quarantined seeds"
                );
            }
        }
    }
}

impl Randomness for FollowerRandomness {
    /// A follower forms no round of its own, but it RECEIVES σ: every certificate
    /// it admits carries the round's seed, and its executor needs that seed to
    /// derive a beacon-active block's `prev_randao`.
    fn record_seed(&self, verified: VerifiedSeed) {
        self.seeds.record(verified);
    }

    fn quarantine_seed(&self, round: Round, seed: BlsSignature) {
        self.seeds.quarantine(round, seed);
    }

    fn on_invalid_seed(&self, epoch: u64) -> InvalidSeed {
        self.keys.on_invalid_seed(epoch)
    }

    fn seed_for(&self, round: Round) -> Option<Seed> {
        self.seeds.lookup(round).map(|signature| Seed {
            target_round: round,
            signature,
        })
    }

    /// The same pin the plane keeps, filed by the same `record`. A follower never
    /// asks for it — it spawns no engine, so it chooses no leader-election base —
    /// but the store beneath it is the real one, and answering from anywhere else
    /// (or answering `None`) would be a claim about the store that is not true.
    fn terminal_seed_at(&self, round: Round) -> Option<Seed> {
        self.seeds.terminal_at(round).map(|signature| Seed {
            target_round: round,
            signature,
        })
    }

    fn seed_edge(&self) -> Arc<Notify> {
        self.seeds.notifier()
    }

    /// Permanently withheld, and by TYPE rather than by state: a follower holds no
    /// share and runs nothing that could produce one. Obtaining `PK_epoch` changes
    /// the VERIFY side only — the group key is not a share.
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

    /// A KEY-ONLY oracle, and by type rather than by state: a follower runs no
    /// ceremony, so it can check an assembled σ against `PK_epoch` and can never
    /// sign, verify or recover a partial.
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        // The beacon-active rule, enforced at the source exactly as the plane
        // enforces it: an oracle tells `verify_certificate` the epoch is
        // beacon-active, so one on a pre-beacon epoch rejects every LEGAL seedless
        // certificate there.
        self.mandatory_at(epoch).then(|| {
            Arc::new(KeyOnlyOracle {
                epoch,
                keys: self.keys.clone(),
                namespace: self.seed_namespace.clone(),
                metrics: self.metrics.clone(),
            }) as Arc<dyn SeedOracle>
        })
    }

    fn ensure_key(&self, epoch: u64, _effort: PinEffort) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            if !self.mandatory_at(epoch) {
                return false;
            }
            // BOTH efforts answer identically, and the asymmetry is the point:
            // `Thorough` has nothing extra to spend here because the upstream rung
            // belongs to the fetch task, off the certificate path. The provenance
            // floor is the plane's, unchanged — a follower has no local
            // reconstruction to floor out today, and a floor that differed by node
            // class is how one of them quietly starts pinning a weaker tier.
            self.keys
                .get_pk(
                    epoch,
                    KeySources {
                        held: Some(&self.held),
                        pull: None,
                        store_floor: Some(KeySource::Carried),
                    },
                )
                .await
                .is_some()
        })
    }

    fn key_edge(&self) -> Arc<Notify> {
        self.keys.notifier()
    }

    fn observe_epoch(&self, _reconciled: Epoch, entered_frontier: Epoch) {
        self.retain_from(
            entered_frontier
                .get()
                .saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64),
        );
    }

    /// The follower's ONLY edge, and it is hot — once per verified certificate,
    /// ~1/s. So it does a cache probe and a non-blocking send and nothing else:
    /// the fetch, the decode and the verification all happen on the fetch task.
    /// A full channel DROPS rather than blocks, which costs nothing because the
    /// next certificate re-asks.
    fn observe_cert(&self, epoch: u64) {
        if self.keys.cached_only(epoch).is_none() {
            let _ = self.want_tx.try_send(epoch);
        }
        self.retain_from(epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        beacon::{
            artifact::decode_artifact,
            dkg_agree::{AgreedArtifact, DkgProposal},
            outcome::DkgOutcome,
            surface::DealtOracle,
        },
        cert_inlet::capture_certificate_seed,
        digest::Digest,
    };
    use alloy_primitives::{Address, B256};
    use commonware_codec::{DecodeExt as _, Encode as _};
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{group::Share, sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_parallel::Sequential;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{
        ordered::{BiMap, Set},
        N3f1, TryCollect as _,
    };
    use fluentbase_bls::{
        beacon::dkg_namespace, fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
        scheme::build_verifier, BlsPubkey, EpochCommittee, PeerPubkey, Scheme as BlsScheme,
    };
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const CHAIN_ID: u64 = 20_994;
    /// The epoch whose committee MINTED the key, so `dkgQual[TARGET]` is set and
    /// the ladder asks for exactly this epoch's artifact.
    const TARGET: u64 = 9;
    const N: usize = 4;

    struct Committee {
        peers: Vec<Ed25519PrivateKey>,
        bls: Vec<ValidatorBlsKeypair>,
    }

    fn committee(seed: u64) -> Committee {
        let mut rng = StdRng::seed_from_u64(seed);
        Committee {
            peers: (0..N)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect(),
            bls: (0..N)
                .map(|_| ValidatorBlsKeypair::generate(&mut rng))
                .collect(),
        }
    }

    impl Committee {
        fn bimap(&self) -> BiMap<PeerPubkey, BlsPubkey> {
            self.peers
                .iter()
                .zip(self.bls.iter())
                .map(|(p, b)| {
                    (
                        p.public_key(),
                        BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
                    )
                })
                .try_collect()
                .expect("unique committee")
        }

        /// Exactly what the follower's own `committee_source` closure hands back.
        fn epoch_committee(&self, epoch: u64) -> EpochCommittee {
            let snap = fluentbase_staking_reader::reader::ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0x11),
                block_number: 4_096,
                epoch,
                validators: self
                    .peers
                    .iter()
                    .zip(self.bls.iter())
                    .enumerate()
                    .map(|(i, (p, b))| ValidatorWithKeys {
                        address: Address::repeat_byte(i as u8 + 1),
                        keys: ConsensusKeys {
                            bls_pubkey: BlsPubkey::decode(b.public_bytes().as_slice())
                                .expect("bls pubkey"),
                            peer_pubkey: p.public_key(),
                            activation_epoch: 0,
                        },
                        tombstoned: false,
                    })
                    .collect(),
                weights: None,
            };
            crate::scheme::epoch_committee_from_snapshot(&snap).expect("committee")
        }

        /// A real DKG over THIS committee's peers, so the artifact the follower
        /// adopts and the σ its certificates carry stand under one key. `deal`
        /// indexes shares by the player's position in the commonware-sorted
        /// `Set`, which is the order `build_signer` asserts a member's share
        /// index against — hence the per-peer lookup rather than a bare `values`.
        fn deal(&self) -> (DkgOutcome, Vec<Share>) {
            let mut rng = StdRng::seed_from_u64(77);
            let players: Set<PeerPubkey> =
                Set::from_iter_dedup(self.peers.iter().map(|k| k.public_key()));
            let (outcome, share_map) =
                deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                    .expect("deal");
            let shares = self
                .peers
                .iter()
                .map(|p| share_map.get_value(&p.public_key()).expect("share").clone())
                .collect();
            (outcome, shares)
        }

        /// A finalization whose certificate CARRIES the round's σ: every signer
        /// holds a threshold share and the assembler recovers the seed into the
        /// cert. It is the only shape in which σ ever reaches a follower.
        fn certify_seeded(
            &self,
            epoch: u64,
            outcome: &DkgOutcome,
            shares: &[Share],
            payload: Digest,
        ) -> Finalization<BlsScheme, Digest> {
            let bimap = self.bimap();
            let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
            let seed_ns = seed_namespace(&fluent_namespace(CHAIN_ID));
            let oracle = |share: Option<Share>| {
                Arc::new(DealtOracle {
                    sharing: outcome.public().clone(),
                    share,
                    namespace: seed_ns.clone(),
                }) as Arc<dyn SeedOracle>
            };
            let round = Round::new(Epoch::new(epoch), View::new(1));
            let proposal = Proposal::new(round, View::new(0), payload);
            let finalizes: Vec<_> = self
                .bls
                .iter()
                .zip(shares)
                .take(3)
                .map(|(kp, share)| {
                    let signer = build_signer(
                        &ns,
                        bimap.clone(),
                        kp,
                        epoch,
                        Some(oracle(Some(share.clone()))),
                    )
                    .expect("member");
                    Finalize::sign(&signer, proposal.clone()).expect("sign")
                })
                .collect();
            Finalization::from_finalizes(
                &build_verifier(&ns, bimap, epoch, Some(oracle(None))),
                finalizes.iter(),
                &Sequential,
            )
            .expect("quorum + recovered seed")
        }

        fn certify(&self, epoch: u64, payload: Digest) -> Finalization<BlsScheme, Digest> {
            let bimap = self.bimap();
            let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
            let round = Round::new(Epoch::new(epoch), View::new(1));
            let proposal = Proposal::new(round, View::new(0), payload);
            let finalizes: Vec<_> = self
                .bls
                .iter()
                .take(3)
                .map(|kp| {
                    let signer = build_signer(&ns, bimap.clone(), kp, epoch, None).expect("member");
                    Finalize::sign(&signer, proposal.clone()).expect("sign")
                })
                .collect();
            Finalization::from_finalizes(
                &build_verifier(&ns, bimap, epoch, None),
                finalizes.iter(),
                &Sequential,
            )
            .expect("quorum")
        }
    }

    fn group_key(seed: u64) -> DkgOutcome {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..N).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
            .expect("deal")
            .0
    }

    fn proposal(epoch: u64) -> DkgProposal {
        proposal_keyed(epoch, group_key(7))
    }

    /// The same proposal carrying a group key the caller chose, so the artifact
    /// the follower adopts is the key its certificates' σ was formed under.
    fn proposal_keyed(epoch: u64, group_key: DkgOutcome) -> DkgProposal {
        DkgProposal {
            target_epoch: epoch,
            logs: (0..N as u8)
                .map(|i| (i, B256::repeat_byte(0x40 + i)))
                .collect(),
            group_key,
            confirms: Vec::new(),
        }
    }

    /// A recording upstream. `served` is swappable so one test can watch what the
    /// follower does with two different answers to the same question.
    #[derive(Clone, Default)]
    struct Upstream {
        served: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
        calls: Arc<AtomicUsize>,
        /// Set for as long as the caller is inside `observe_cert`. The fetch
        /// asserting on it is what makes "never inline" a real observation rather
        /// than a claim about a signature.
        forbidden: Arc<AtomicBool>,
    }

    impl Upstream {
        fn fetch(&self) -> ArtifactFetch {
            let me = self.clone();
            Arc::new(move |_epoch: u64| {
                assert!(
                    !me.forbidden.load(Ordering::SeqCst),
                    "the fetch ran on the caller's task — observe_cert must only probe and send"
                );
                me.calls.fetch_add(1, Ordering::SeqCst);
                let served = me.served.lock().expect("served").clone();
                let forbidden = me.forbidden.clone();
                Box::pin(async move {
                    assert!(
                        !forbidden.load(Ordering::SeqCst),
                        "the fetch future was polled on the caller's task"
                    );
                    served
                }) as BoxFuture<'static, _>
            })
        }
    }

    fn config(c: &Committee, up: &Upstream) -> FollowerRandomnessConfig {
        let committee = c.epoch_committee(TARGET);
        FollowerRandomnessConfig {
            chain_id: CHAIN_ID,
            committees: Arc::new(move |epoch: u64| (epoch == TARGET).then(|| committee.clone())),
            // Only TARGET minted; every epoch above it carries TARGET's key.
            dkg_qual: Arc::new(|epoch: u64| Some(epoch == TARGET)),
            fetch: up.fetch(),
        }
    }

    /// Let the fetch task run to the point where `pred` holds. Bounded so a
    /// regression fails the assertion below instead of hanging the suite.
    async fn settle(ctx: &deterministic::Context, pred: impl Fn() -> bool) {
        for _ in 0..64 {
            if pred() {
                return;
            }
            ctx.sleep(Duration::from_millis(1)).await;
        }
    }

    /// A follower must not trust its upstream for the key any more than for a
    /// certificate. An artifact carrying a quorum of the WRONG committee is
    /// refused, nothing is stored, and the epoch stays unpinned — i.e. its certs
    /// keep taking vote-only admission rather than being verified against a key
    /// the upstream chose.
    ///
    /// The forged bytes are asserted to DECODE first, so the refusal is proven to
    /// come from the committee check and not from the codec.
    #[test]
    fn an_artifact_certified_by_the_wrong_committee_is_refused() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let foreign = committee(2);
            let body = proposal(TARGET);
            let forged: AgreedArtifact = (body.clone(), foreign.certify(TARGET, body.digest()));
            let bytes = forged.encode().to_vec();
            assert!(
                decode_artifact(&bytes).is_ok(),
                "the forgery must survive the codec, or this tests the decoder"
            );

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(bytes);
            let fb = for_follower(&ctx, config(&c, &up));

            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || up.calls.load(Ordering::SeqCst) > 0).await;

            assert!(
                up.calls.load(Ordering::SeqCst) > 0,
                "the fetch must have run"
            );
            assert!(
                !fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "a refused artifact must leave the epoch KEYLESS"
            );
            assert!(
                (fb.artifact_bytes)(TARGET).is_none(),
                "a refused artifact must not be stored, let alone re-served to a tier-2 follower"
            );
        });
    }

    /// The positive half: a genuine artifact is adopted, the epoch's key then
    /// resolves, and it resolves WITHOUT a fetch — the certificate path stays
    /// network-free after the one off-path delivery.
    #[test]
    fn a_genuine_artifact_is_adopted_and_then_resolves_without_a_fetch() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let body = proposal(TARGET);
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));
            let expected = *crate::beacon::outcome::group_public_key(&genuine.0.group_key);

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = for_follower(&ctx, config(&c, &up));

            assert!(
                !fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "nothing is held before the first delivery"
            );
            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || (fb.artifact_bytes)(TARGET).is_some()).await;

            let after_adoption = up.calls.load(Ordering::SeqCst);
            assert_eq!(after_adoption, 1, "exactly one delivery");
            assert!(fb.randomness.ensure_key(TARGET, PinEffort::Local).await);
            // The VALUE, read off the adopted artifact itself: `ensure_key` reports
            // only that a key resolved, so without this the test would pass on a
            // wrong one.
            assert_eq!(
                *crate::beacon::outcome::group_public_key(
                    &decode_artifact(&(fb.artifact_bytes)(TARGET).expect("adopted"))
                        .expect("decodes")
                        .0
                        .group_key
                ),
                expected,
                "the adopted PK_epoch is the genuine one"
            );
            // A carried (stable) epoch above the mint resolves off the SAME
            // artifact through the dkgQual walk — no second delivery.
            assert!(
                fb.randomness.ensure_key(TARGET + 3, PinEffort::Local).await,
                "a stable epoch carries the minting epoch's key"
            );
            // And the trigger stands down: the epoch is cached, so no further
            // certificate re-asks for it.
            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || up.calls.load(Ordering::SeqCst) > after_adoption).await;
            assert_eq!(
                up.calls.load(Ordering::SeqCst),
                after_adoption,
                "ensure_key and observe_cert must not spend the network once the key is held"
            );
            assert!(
                (fb.artifact_bytes)(TARGET).is_some(),
                "the adopted artifact is servable to a tier-2 follower"
            );
        });
    }

    /// `observe_cert` runs once per verified certificate (~1/s) on the task that
    /// drains the cert stream. It must do nothing but a cache probe and a
    /// non-blocking send: the fetch, the decode and the verify all belong to the
    /// background task. The `forbidden` flag is what proves it — the fetch panics
    /// if it is reached while the caller is still inside `observe_cert`.
    #[test]
    fn observe_cert_neither_blocks_nor_fetches_inline() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let body = proposal(TARGET);
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = for_follower(&ctx, config(&c, &up));

            up.forbidden.store(true, Ordering::SeqCst);
            fb.randomness.observe_cert(TARGET);
            assert_eq!(
                up.calls.load(Ordering::SeqCst),
                0,
                "observe_cert returned having already fetched — it is on the hot path"
            );
            up.forbidden.store(false, Ordering::SeqCst);

            settle(&ctx, || up.calls.load(Ordering::SeqCst) > 0).await;
            assert_eq!(
                up.calls.load(Ordering::SeqCst),
                1,
                "the work happens on the background task, and it does happen"
            );
        });
    }

    /// An upstream that serves nothing — no artifact, or a server too old to know
    /// the method, which reach here identically as `None` — must leave the
    /// follower exactly where it was: unpinned, still asking, and with nothing
    /// stored. It is never a data fault and never poisons the epoch.
    #[test]
    fn an_upstream_with_no_artifact_leaves_the_epoch_unpinned_and_retryable() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let up = Upstream::default();
            let fb = for_follower(&ctx, config(&c, &up));

            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || up.calls.load(Ordering::SeqCst) > 0).await;

            assert_eq!(up.calls.load(Ordering::SeqCst), 1);
            assert!(
                !fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "no artifact means no key — never a wrong one"
            );
            assert!((fb.artifact_bytes)(TARGET).is_none());

            // The miss is not memoised: once the upstream has it, the very next
            // want adopts it. (The per-epoch throttle bounds HOW OFTEN, and it is
            // the only thing between these two wants.)
            let body = proposal(TARGET);
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            ctx.sleep(PULL_MIN_INTERVAL).await;
            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || (fb.artifact_bytes)(TARGET).is_some()).await;
            assert!(
                fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "a miss must not be terminal"
            );
        });
    }

    /// σ reaches this node class on the certificate and nowhere else — a follower
    /// forms no round and has no by-round transport — and its executor derives a
    /// beacon-active block's `prev_randao` from that σ alone. So a checked σ has
    /// to be filed and served back, and the seed edge has to be the store's
    /// rather than the `idle` handle nothing ever fires.
    #[test]
    fn a_verified_certificates_seed_is_filed_and_served() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = for_follower(&ctx, config(&c, &up));

            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || (fb.artifact_bytes)(TARGET).is_some()).await;
            assert!(fb.randomness.ensure_key(TARGET, PinEffort::Local).await);

            let cert = c.certify_seeded(TARGET, &outcome, &shares, Digest(B256::repeat_byte(0xcc)));
            let round = cert.proposal.round;
            let sigma = cert
                .certificate
                .seed()
                .expect("a beacon-active certificate carries the round seed");

            let edge = fb.randomness.seed_edge();
            capture_certificate_seed(fb.randomness.as_ref(), round, &cert);

            assert_eq!(
                fb.randomness.seed_for(round).map(|s| s.signature),
                Some(sigma),
                "the σ the certificate carried is what a later derive reads back"
            );
            let woken = tokio::select! {
                _ = edge.notified() => true,
                _ = ctx.sleep(Duration::from_millis(10)) => false,
            };
            assert!(
                woken,
                "a held tip waits on the seed edge; an unfired one parks it forever"
            );
        });
    }

    /// The keyless window is the ORDINARY state here: a follower obtains
    /// `PK_epoch` only by fetching the epoch's artifact, so σ routinely lands
    /// first. Held rather than dropped, and re-checked when the key turns up —
    /// an unwired quarantine would discard most of what the cert doors file, in
    /// silence.
    #[test]
    fn a_seed_that_arrives_before_the_key_is_held_and_then_promoted() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            // Nothing served yet: the follower is keyless, which is where it
            // spends the opening of every epoch.
            let up = Upstream::default();
            let fb = for_follower(&ctx, config(&c, &up));

            let cert = c.certify_seeded(TARGET, &outcome, &shares, Digest(B256::repeat_byte(0xcc)));
            let round = cert.proposal.round;
            let sigma = cert
                .certificate
                .seed()
                .expect("a beacon-active certificate carries the round seed");
            capture_certificate_seed(fb.randomness.as_ref(), round, &cert);
            assert!(
                fb.randomness.seed_for(round).is_none(),
                "an unchecked σ must never reach the served map"
            );

            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            fb.randomness.observe_cert(TARGET);
            settle(&ctx, || fb.randomness.seed_for(round).is_some()).await;
            // Only a σ that was HELD can be served now: the capture above ran
            // once, and nothing re-delivers it.
            assert_eq!(
                fb.randomness.seed_for(round).map(|s| s.signature),
                Some(sigma),
                "the key landing must release what the keyless window held"
            );
        });
    }

    /// The beacon-active rule, which binds every implementation: an ORACLE tells
    /// `verify_certificate` that the epoch is beacon-active, so one on a
    /// pre-beacon epoch rejects every LEGAL seedless certificate there.
    #[test]
    fn a_pre_beacon_epoch_gets_no_oracle_and_no_key() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let up = Upstream::default();
            let fb = for_follower(&ctx, config(&c, &up));
            for epoch in 0..super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH {
                assert!(
                    fb.randomness.oracle_for(epoch).is_none(),
                    "epoch {epoch} predates the beacon: an oracle there would reject \
                     every legal seedless certificate"
                );
                assert!(!fb.randomness.ensure_key(epoch, PinEffort::Local).await);
                assert!(!fb.randomness.mandatory_at(epoch));
            }
        });
    }
}
