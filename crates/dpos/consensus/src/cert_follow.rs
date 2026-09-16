//! Cert-follow transport seam.
//!
//! The follower engine is transport-agnostic: it pulls finalizations by height
//! (the marshal's gap-repair path) and consumes a live stream of finalized certs.
//! The concrete WS client lives in the node crate, which decodes the hex
//! `CertifiedBlock` at the crate boundary, so `consensus` never names node RPC
//! types.
//!
//! A non-validator follower is a near-planeless layer (inlet + executor, the
//! executor being the sole reth writer); an upstream-configured validator runs the
//! cert-inlet as a second producer into its own marshal. Both share only
//! [`CertUpstream`] / [`UpstreamFinalized`].

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

/// A finalized certificate and its block, already decoded from the upstream
/// `consensus` RPC wire form. The driver verifies the certificate against the
/// per-epoch scheme — the trustless gate.
#[derive(Clone)]
pub struct UpstreamFinalized {
    /// The finalization certificate (2f+1 BLS multisig over `block`'s digest).
    pub finalization: Finalization<BlsScheme, Digest>,
    /// The finalized ordering artifact the certificate commits to. Its `result`
    /// field is the only real EVM hash the follower can use to drive reth EL-sync at
    /// cold-start (the digest is an ordering digest).
    pub block: OrderBlock,
}

/// How a by-height walk over every configured upstream ended
/// ([`CertUpstream::get_finalization_everywhere`]).
///
/// The two negatives are different facts, and only the walk can tell them apart:
/// crash-survivor recovery reads "nobody holds it" as local consensus data loss and
/// tells the operator to re-sync the EL disk from a snapshot, which is irreversible,
/// so it may only ever be told that by a walk that got answers.
pub enum WalkOutcome {
    Got(Box<UpstreamFinalized>),
    /// Every configured upstream answered, and none of them holds the height.
    /// This — and only this — is evidence about the record.
    MissedEverywhere,
    /// Not one configured upstream answered at all: unreachable, a transport
    /// failure, or a plane fetch that timed out. Evidence about the link only.
    NoneAnswered,
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

    /// [`Self::get_finalization`], but on an explicit content miss it asks the rest
    /// of the configured sources before giving up, and says which negative it got.
    ///
    /// The default is one plain `get_finalization`, whose `None` cannot tell a
    /// content miss from a dead link and so is [`WalkOutcome::NoneAnswered`] — the
    /// fail-safe reading. Correct for the plane: a registered node's resolver walks
    /// peers on its existing connections. Only the WS handle can answer
    /// [`WalkOutcome::MissedEverywhere`], because only it asks a list of servers.
    ///
    /// Use this only where a miss is semantically expensive and the caller runs on
    /// its own cadence: boundary seeding at boot, the re-jump landing, crash
    /// recovery. Not for the marshal's gap repair, which issues up to `MAX_REPAIR`
    /// concurrent by-height pulls per sweep — a walk there is
    /// `MAX_REPAIR × (urls − 1)` connections per second, forever, for a height
    /// nobody holds.
    fn get_finalization_everywhere(
        &self,
        height: Height,
    ) -> impl Future<Output = WalkOutcome> + Send {
        async move {
            match self.get_finalization(height).await {
                Some(uf) => WalkOutcome::Got(Box::new(uf)),
                None => WalkOutcome::NoneAnswered,
            }
        }
    }

    /// Fetch the upstream's latest finalized block. Used at cold-start to obtain a
    /// (trusted, for EL-sync only) head to drive reth's backward sync into the DPoS
    /// era. The head hash is the only trusted input: every cert from the anchor
    /// forward is verified by the driver, which transitively authenticates the anchor
    /// hash. Closing the head-hash trust is the deferred L1 anchor source.
    fn get_latest(&self) -> impl Future<Output = Option<UpstreamFinalized>> + Send;

    /// Fetch the agreed epoch-key artifact minted at `epoch`, as the wire bytes the
    /// beacon's [`crate::beacon::ArtifactFetch`] hands in. `None` covers every
    /// negative alike: the upstream holds none, it is too old to know the method, or
    /// the link is down — all mean "stay unpinned and ask again", never a data fault.
    ///
    /// The default `None` is correct for the plane: a registered node pulls artifacts
    /// from its committee peers and has no use for this route. Only the WS handle
    /// overrides it, because a follower is bootstrapper-less with an ephemeral
    /// identity.
    ///
    /// The answer is not trusted: it is checked against `committee[epoch]` from the
    /// caller's own chain state before anything is kept.
    fn get_epoch_artifact(&self, _epoch: u64) -> impl Future<Output = Option<Vec<u8>>> + Send {
        std::future::ready(None)
    }

    /// Drop the current connection and move to the next configured upstream URL.
    /// Called when the current upstream served a tampered or mismatched cert;
    /// connection-level failures rotate inside the transport actor.
    fn rotate(&self) -> impl Future<Output = ()> + Send;

    /// Boxed [`crate::cert_inlet::RotateUpstream`] over [`Self::rotate`], for wiring
    /// into `CertInlet::with_rotate`. The inlet's data-fault trigger is a boxed
    /// closure so the inlet gains no `U: CertUpstream` generic; this is the one place
    /// that boxing lives.
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
/// Erased because its dependencies all live at launch in `dpos.rs`, while the two
/// callers must not grow `U: CertUpstream` / `C: CommitteeSource` generics to reach
/// them. `None` at a call site means "no upstream configured".
pub type BoundaryFetchFn =
    Arc<dyn Fn(u64, B256) -> BoxFuture<'static, Option<UpstreamFinalized>> + Send + Sync>;

/// Authenticated by-height fetch of one finalized block, for seeding an epoch
/// boundary that a jump left below the marshal floor.
///
/// Same verification as the cold-start landing: structural (`payload == digest`)
/// plus a 2f+1 BLS multisig against `committee[E]` read at `at_hash`. Two deliberate
/// differences:
///
/// - The height is pinned. `verify_jump_structural` only ties the cert to the block
///   it arrived with, and `verify_jump_authenticated` takes the epoch from the
///   cert's own round, so without this check a valid finalization for a different
///   height would be stored under the index asked for.
/// - `l1_checkpoint` is deliberately not forwarded: an unreadable committee is
///   success for the landing, whose ancestry the checkpoint authenticates, but
///   unsound for an arbitrary older height, so here it is failure.
///
/// Soft-fail by contract: `None` leaves the member verify-only for its landing
/// epoch, so an absent, slow or hostile upstream can never leave the node worse off.
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

    // `_everywhere`: this seam serves the semantic callers — boundary seeding at
    // boot, the re-jump landing, and the beacon-key repair rung — each of which pays
    // for a miss with an epoch of verify-only. The marshal's gap repair deliberately
    // does not come through here.
    let uf = match upstream
        .get_finalization_everywhere(Height::new(height))
        .await
    {
        WalkOutcome::Got(uf) => *uf,
        WalkOutcome::MissedEverywhere => return failed("no configured upstream holds the height"),
        WalkOutcome::NoneAnswered => return failed("no configured upstream answered"),
    };
    if uf.block.height != height {
        return failed("upstream served a different height than requested");
    }
    if let Err(e) = verify_jump_structural(&uf) {
        warn!(height, error = %e, "boundary finalization is malformed");
        return failed("cert payload != block digest");
    }
    if let Err(e) = verify_jump_authenticated(&uf, committees, at_hash, ctx) {
        warn!(height, error = %e, "boundary finalization failed BLS authentication");
        return failed("committee unreadable or multisig invalid");
    }
    Some(uf)
}
