//! Glue stubs the follower marshal assembly needs to typecheck.
//!
//! A follower never broadcasts blocks, never tracks a p2p peer set, and has no
//! `FluentApp` proposer — but the marshal + epoch machinery are generic over
//! those seams. These three stubs satisfy the type bounds with no-ops:
//!
//! 1. [`null_broadcast`] — an unused `buffered::Mailbox` (tempo `follow/stubs.rs`).
//! 2. [`AppReporter`] — forwards the marshal's `Update<Block>` straight to the
//!    executor (`FluentApp` is the validator-side `Reporter`; the follower has no
//!    app, so this thin newtype takes its place — application.rs:311).
//! 3. [`NullPeerSetSink`] — a no-op [`PeerSetSink`] for the observer-only
//!    `EpochTransition` (the validator wires a live `OracleHandle`; the follower
//!    tracks no peers).

use crate::{block::Block, executor};
use commonware_broadcast::buffered;
use commonware_consensus::{marshal::Update, Reporter};
use commonware_cryptography::{
    ed25519::{PrivateKey, PublicKey},
    Signer as _,
};
use commonware_math::algebra::Random as _;
use commonware_p2p::utils::StaticProvider;
use commonware_runtime::{BufferPooler, Clock, Metrics, Spawner};
use commonware_utils::ordered::Set;
use fluentbase_bls::PeerPubkey;
use fluentbase_staking_reader::PeerSetSink;
use rand_08::SeedableRng as _;
use reth_ethereum_engine_primitives::EthPayloadAttributes;
use std::future::Future;

/// A never-used broadcast mailbox. The follower has no consensus peers to
/// broadcast to; the marshal's gap-repair is served entirely by the
/// follower's resolver (upstream WS + local reth). Mirrors tempo
/// `follow/stubs.rs::null_broadcast`.
pub(super) fn null_broadcast<E: Clock + Spawner + Metrics + BufferPooler>(
    context: E,
    mailbox_size: usize,
) -> buffered::Mailbox<PublicKey, Block> {
    // Deterministic throwaway key for the unused engine (seed 0 — never signs).
    let mut rng = rand_08::rngs::StdRng::seed_from_u64(0);
    let private_key = PrivateKey::random(&mut rng);
    let public_key = private_key.public_key();

    let config = buffered::Config {
        public_key,
        mailbox_size,
        deque_size: 0,
        priority: false,
        codec_config: (),
        peer_provider: StaticProvider::new(0, Set::default()),
    };

    let (_engine, mailbox) = buffered::Engine::new(context, config);
    mailbox
}

/// Forwards every marshal `Update<Block>` to the executor's `Command::Finalize`.
/// The validator path uses `FluentApp` as this `Reporter` (application.rs:311);
/// the follower has no app, so this newtype carries the executor mailbox alone.
#[derive(Clone)]
pub(super) struct AppReporter {
    executor: executor::Mailbox<EthPayloadAttributes>,
}

impl AppReporter {
    pub(super) fn new(executor: executor::Mailbox<EthPayloadAttributes>) -> Self {
        Self { executor }
    }
}

impl Reporter for AppReporter {
    type Activity = Update<Block>;

    async fn report(&mut self, activity: Update<Block>) {
        // The `Exact` ack rides inside `Update::Block`; the executor fires it after
        // EL `new_payload`. A closed mailbox means the executor task already exited —
        // the dropped ack trips the marshal's `PendingAcks` supervisor cascade
        // (same recovery path as the validator), so just log the send failure.
        if let Err(e) = self.executor.send(executor::Message {
            cause: tracing::Span::current(),
            command: executor::Command::Finalize(Box::new(activity)),
        }) {
            tracing::error!(
                ?e,
                "follower executor mailbox closed; finalize command dropped"
            );
        }
    }
}

/// A [`PeerSetSink`] that discards the committee. The follower runs
/// `EpochTransition` only to detect boundaries + surface the frozen committee
/// snapshot (via its `boundary_tx`) for BLS-verifier registration — it tracks no
/// p2p peers, so the sink is a no-op.
#[derive(Clone)]
pub(super) struct NullPeerSetSink;

impl PeerSetSink for NullPeerSetSink {
    fn track(&mut self, _epoch: u64, _peers: Set<PeerPubkey>) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}
