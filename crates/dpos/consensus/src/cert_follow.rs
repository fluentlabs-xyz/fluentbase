//! Cert-follow transport seam (post unified-node-mode collapse).
//!
//! The follower engine is transport-agnostic: it pulls finalizations by height
//! (the marshal's gap-repair path) and consumes a live stream of finalized
//! certs. The concrete WS client lives in the **node** crate (jsonrpsee), which
//! decodes the hex `CertifiedBlock` at the crate boundary so `consensus` never
//! names node RPC types. This mirrors tempo's `follow/upstream` trait seam
//! (`UpstreamActor`, follow/upstream/mod.rs:22), adapted to fluentbase's
//! consensus/node crate split.
//!
//! A non-validator follower is a near-planeless
//! [`crate::dpos::DposLayer::launch_follower`] (inlet + executor, the executor
//! being the sole reth writer); an upstream-configured validator runs the
//! cert-inlet as a second producer into its own marshal. Both share only
//! [`CertUpstream`] / [`UpstreamFinalized`] — the by-height pull + live
//! finalized-cert stream the node's WS actor implements (the inlet's sole
//! producer; the cold-start JUMP's `get_latest` source). The per-epoch BLS
//! verifier read ([`crate::cert_inlet::RethCommitteeSource`]) and the EL-sync
//! JUMP ([`crate::cold_start_jump::RethElSync`]) live in their own modules.

use crate::{
    cert_inlet::CommitteeSource,
    cold_start_jump::{verify_jump_authenticated, verify_jump_structural},
    digest::Digest,
    order_block::OrderBlock,
    sync_metrics::SyncMetrics,
};
use alloy_primitives::B256;
use commonware_consensus::{simplex::types::Finalization, types::Height};
use commonware_runtime::Clock;
use fluentbase_bls::Scheme as BlsScheme;
use futures::future::BoxFuture;
use rand_core::CryptoRngCore;
use std::{future::Future, sync::Arc};
use tracing::warn;

/// A finalized certificate + its block, already decoded from the upstream
/// `consensus` RPC wire form. The node-side WS actor decodes the hex
/// `CertifiedBlock` into this before handing it to the engine; the driver then
/// verifies the certificate against the per-epoch `EpochSchemeProvider` (the
/// trustless gate — a tampered cert never finalizes).
#[derive(Clone)]
pub struct UpstreamFinalized {
    /// The finalization certificate (2f+1 BLS multisig over `block`'s digest).
    pub finalization: Finalization<BlsScheme, Digest>,
    /// The finalized ordering artifact the certificate commits to. Its
    /// `result` field is the only REAL EVM hash the follower can use to drive
    /// reth EL-sync at cold-start (the digest is an ordering digest).
    pub block: OrderBlock,
}

/// By-height pull seam for the marshal's gap-repair resolver. `Clone` so the
/// resolver can fan out concurrent fetches; the concrete impl is the node's WS
/// upstream mailbox.
pub trait CertUpstream: Clone + Send + Sync + 'static {
    /// Fetch the finalization + block at `height`, or `None` if the upstream
    /// does not (yet) have it.
    fn get_finalization(
        &self,
        height: Height,
    ) -> impl Future<Output = Option<UpstreamFinalized>> + Send;

    /// Fetch the upstream's latest finalized block. Used at cold-start to obtain a
    /// (trusted, for EL-sync only) head to drive the follower's reth devp2p
    /// backward-sync into the DPoS era. The head *hash* is the only trusted input:
    /// every cert from the anchor forward is cryptographically verified by the driver,
    /// which transitively authenticates the anchor's hash. Closing the head-hash trust
    /// is the deferred L1 anchor source.
    fn get_latest(&self) -> impl Future<Output = Option<UpstreamFinalized>> + Send;

    /// Drop the current connection and move to the next configured upstream
    /// URL. Called by the follow loop when the CURRENT upstream served
    /// unverifiable data (tampered/mismatched cert) — connection-level
    /// failures rotate inside the transport actor on their own.
    fn rotate(&self) -> impl Future<Output = ()> + Send;

    /// Boxed [`crate::cert_inlet::RotateUpstream`] over [`Self::rotate`], for
    /// wiring into `CertInlet::with_rotate`. The inlet's data-fault rotation
    /// trigger is a `Box`ed closure (so the inlet gains no `U: CertUpstream`
    /// generic); this is the one place that boxing lives — both the node-side
    /// `spawn_cert_inlet` and the follower's inline inlet build their trigger
    /// from this default method instead of hand-rolling the same `Arc::new(move
    /// || Box::pin(async move { up.rotate().await }))`.
    fn rotate_callback(&self) -> crate::cert_inlet::RotateUpstream {
        let up = self.clone();
        std::sync::Arc::new(move || {
            let up = up.clone();
            Box::pin(async move { up.rotate().await }) as futures::future::BoxFuture<'static, ()>
        })
    }
}

/// Erased by-height seam for seeding an epoch-boundary block that sits below the
/// marshal floor: `(height, at_hash) -> verified finalization+block`.
///
/// Erased because its dependencies (the upstream, the committee source, a runtime
/// context, the metrics) all live at launch in `dpos.rs`, while the two callers —
/// the cold-start boot in `outer.rs` and `executor::reseed_forward` — must not grow
/// `U: CertUpstream` / `C: CommitteeSource` generics to reach them. `None` at a call
/// site means "no upstream configured".
pub type BoundaryFetchFn =
    Arc<dyn Fn(u64, B256) -> BoxFuture<'static, Option<UpstreamFinalized>> + Send + Sync>;

/// Authenticated by-height fetch of one finalized block, for seeding an epoch
/// boundary that a jump left below the marshal floor.
///
/// Same seam and same verification as the cold-start jump landing itself:
/// structural (`payload == digest`) plus a 2f+1 BLS multisig against `committee[E]`
/// read at `at_hash`. Two deliberate differences from the landing, both load-bearing:
///
/// - **The height is pinned.** Nothing else binds the response to the request:
///   `verify_jump_structural` only ties the cert to the block it arrived with, and
///   `verify_jump_authenticated` takes the epoch from the cert's own round. Without
///   this check a valid finalization for a DIFFERENT height passes everything and is
///   then stored under the index we asked for.
/// - **`l1_checkpoint` is deliberately not forwarded.** `verify_jump_authenticated`
///   treats an unreadable committee as success when an L1 checkpoint is configured.
///   That is sound for the LANDING, whose ancestry the checkpoint authenticates, and
///   unsound for an arbitrary older height. Here an unreadable committee is failure.
///
/// Soft-fail by contract: `None` leaves the member verify-only for its landing epoch
/// — exactly the behaviour that existed before boundary seeding — so an absent, slow
/// or hostile upstream can never leave the node worse off than it is today. Every
/// failure is counted and logged with that consequence named.
pub(crate) async fn fetch_verified_boundary<U, C>(
    upstream: &U,
    committees: &C,
    ctx: &mut (impl Clock + CryptoRngCore),
    metrics: &SyncMetrics,
    at_hash: B256,
    height: u64,
) -> Option<UpstreamFinalized>
where
    U: CertUpstream,
    C: CommitteeSource,
{
    let failed = |reason: &str| {
        metrics.jump_boundary_refetch_failed.inc();
        warn!(
            height,
            reason,
            "epoch-boundary block could not be seeded below the marshal floor — this member \
             stays verify-only (no proposals, no votes) until the next epoch boundary"
        );
        None
    };

    let Some(uf) = upstream.get_finalization(Height::new(height)).await else {
        return failed("upstream does not serve the height");
    };
    if uf.block.height != height {
        return failed("upstream served a different height than requested");
    }
    if let Err(e) = verify_jump_structural(&uf) {
        warn!(height, error = %e, "boundary finalization is malformed");
        return failed("cert payload != block digest");
    }
    if let Err(e) = verify_jump_authenticated(&uf, committees, at_hash, None, ctx) {
        warn!(height, error = %e, "boundary finalization failed BLS authentication");
        return failed("committee unreadable or multisig invalid");
    }
    Some(uf)
}
