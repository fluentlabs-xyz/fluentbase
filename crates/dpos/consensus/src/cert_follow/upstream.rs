//! Upstream abstraction for the cert-follower engine.
//!
//! The follower engine is transport-agnostic: it pulls finalizations by height
//! (the marshal's gap-repair path) and consumes a live stream of finalized
//! certs. The concrete WS client lives in the **node** crate (jsonrpsee), which
//! decodes the hex `CertifiedBlock` at the crate boundary so `consensus` never
//! names node RPC types. This mirrors tempo's `follow/upstream` trait seam
//! (`UpstreamActor`, follow/upstream/mod.rs:22), adapted to fluentbase's
//! consensus/node crate split.

use crate::{digest::Digest, order_block::OrderBlock};
use commonware_consensus::{simplex::types::Finalization, types::Height};
use fluentbase_bls::Scheme as BlsScheme;
use std::future::Future;

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
