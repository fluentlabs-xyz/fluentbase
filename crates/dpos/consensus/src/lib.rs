//! Fluent DPoS consensus: Commonware Simplex ⇄ Reth integration.
//!
//! Built as an **injection-style library** — collaborators are constructor
//! params, not awaited. Pinned against commonware `monorepo @ v2026.4.0`.
//!
//! Frozen leaves (collaborator-free):
//!   [`digest`] (the OrderBlock keccak digest wrapper),
//!   [`weighted_vrf`] (stake-weighted VRF leader elector),
//!   [`timeouts`] (timeout family + commonware panic-invariant guard).
//!
//! Wiring layer ([`application`], [`order_block`], [`engine`], [`epoch_manager`],
//!   [`outer`], [`executor`], [`slasher`]): adapts upstream commonware
//!   Simplex behaviour to the Fluent Reth execution layer.

/// Trailing epochs retained for cross-epoch certificate verification: the BLS
/// schemes in [`outer::EpochSchemeProvider`], and — because a key is only ever
/// read to verify a certificate a retained scheme covers — every beacon window
/// that has to reach as far back as those schemes do.
///
/// The beacon's DERIVED key tiers, which this used to name, are deleted (П-3): the
/// artifact store owns `PK_epoch` and has NO retention at all, deliberately (see
/// `beacon::artifact`'s module doc — on a long-stable committee the OLDEST record
/// is the valuable one, so any epoch-measured window would drop it first). What
/// does measure itself against this number is the recompute-heal and DKG-journal
/// window (`beacon::JOURNAL_RETENTION_EPOCHS`, one alias of this value), the
/// mint-artifact acquisition window, the seed index's terminal-round protection, and the
/// committee module's read window — and the last two of those are coupled: the
/// acquisition asks for exactly the epochs the committee module can still answer a
/// `changed` bit for.
///
/// Crate-level because it is one policy with enforcers on opposite sides of the
/// randomness surface. Defining it in either of them would make that one
/// import the other, which is exactly the coupling the surface exists to remove.
///
/// (Marshal backfill / catch-up register epochs in order, so older schemes are
/// never re-read once the frontier passes them.)
pub const SCHEME_RETENTION_EPOCHS: usize = 8;

pub mod application;
pub mod beacon;
/// DEVNET/TEST-ONLY byzantine code — the single home for all byzantine logic
/// (`ByzantineMode`, `forge_outcome_same_committee`, `VoteEquivocator`). Gated
/// behind `dpos-devnet-byzantine`; also built under `test` so in-crate forge/certify
/// unit tests reach the helpers. Never compiled into a production build.
#[cfg(any(test, feature = "dpos-devnet-byzantine"))]
pub mod byzantine;
pub mod cert_follow;
pub mod cert_inlet;
pub mod cold_start_jump;
pub mod committee;
pub mod digest;
pub mod dpos;
pub mod engine;
pub mod epoch_manager;
pub mod epocher;
/// Materialized-state-gated executed-hash probe shared by both
/// [`fluentbase_staking_reader::EpochTransition`] closures (signer + beacon).
pub mod executed;
pub mod executor;
pub mod extra_data;
/// The executor fault taxonomy (family 5): one closed [`fault::FaultClass`]
/// vocabulary the leaf error-mappers speak, replacing per-site behavioral
/// classifiers. See the module docs.
pub mod fault;
pub mod feed_sink;
pub mod order_block;
pub mod outer;
pub mod plane_upstream;
pub mod scheme;
pub mod slasher;
pub mod spec_exec;
pub mod sync_metrics;
#[cfg(test)]
mod testbed;
pub mod timeouts;
/// Stake-weighted VRF leader elector (used by [`engine`], same crate).
mod weighted_vrf;

pub use application::{
    gas_limit_within_1_1024, step_gas_limit, BeaconEngineLike, DerivedBlock, DerivedBlockBuilder,
    ExecutedChain, FinalizedCursor, FluentApp, OrderingAssembler, ParentHeaderMissing,
    VERIFY_EXEC_BUDGET,
};
pub use cert_follow::{CertUpstream, UpstreamFinalized};
pub use cert_inlet::{
    CertInlet, CommitteeSource, MarshalSink, NoopResolver, RethCommitteeSource, RotateUpstream,
    MAX_UPSTREAM_FAULTS,
};
pub use cold_start_jump::{assert_l1_checkpoint, ElSync, RethElSync, JUMP_THRESHOLD};
// `Geometry` is deliberately NOT here: since the store took its geometry lazily
// (a `GeometryRx`), the type has no user outside `committee/` — re-exporting it
// would be public surface nobody can reach through.
pub use committee::{
    epoch_verifier, Anchor, BeaconSlot, Committee, CommitteeError, CommitteeReadsFacade,
    CommitteeRecord, CommitteeStore, EpochReads, EpochVerifier, GeometryRx, Member, RethAnchor,
};
pub use digest::Digest;
pub use dpos::{
    peek_consensus_archive_last_finalized, DposLayer, DposLayerConfig, DposLayerHandle,
    FollowerLayerConfig, FollowerRethHandle, PlaneMux, RethHandle, SharedBeaconPlane,
};
pub use epocher::OriginEpocher;
pub use executed::executed_state_hash;
pub use fault::{DeferReason, EngineError, Fault, FaultClass};
pub use feed_sink::FeedSink;
pub use order_block::{
    anchor_order_block, result_final_height, result_target, OrderBlock, ResultTarget, K,
};
pub use outer::{MarshalMailbox, OuterBuilder, OuterEngine};
pub use plane_upstream::{FrontierKey, PlaneUpstreamHandle};
pub use timeouts::ConsensusTimeouts;

/// commonware journal replay buffer (per partition). Shared by the validator
/// marshal archives ([`outer`]) and the per-epoch Simplex engines ([`engine`]);
/// they MUST stay identical or storage replay desynchronizes between the two.
pub(crate) const REPLAY_BUFFER: std::num::NonZeroUsize =
    commonware_utils::NZUsize!(8 * 1024 * 1024);
/// commonware journal write buffer (per partition). Same shared-identity
/// constraint as [`REPLAY_BUFFER`].
pub(crate) const WRITE_BUFFER: std::num::NonZeroUsize = commonware_utils::NZUsize!(1024 * 1024);
