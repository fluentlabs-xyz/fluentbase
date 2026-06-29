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
pub mod digest;
pub mod dpos;
pub mod engine;
pub mod epoch_manager;
pub mod epocher;
pub mod executor;
pub mod extra_data;
pub mod feed_sink;
pub mod order_block;
pub mod outer;
pub mod scheme;
pub mod slasher;
pub mod spec_exec;
pub mod timeouts;
/// Stake-weighted VRF leader elector (used only by [`engine`], same crate).
mod weighted_vrf;

pub use application::{
    gas_limit_within_1_1024, step_gas_limit, BeaconEngineLike, DerivedBlock, DerivedBlockBuilder,
    ExecutedChain, FluentApp, OrderingAssembler, ParentHeaderMissing, VERIFY_EXEC_BUDGET,
};
pub use cert_follow::{CertUpstream, UpstreamFinalized};
pub use cert_inlet::{
    CertInlet, CommitteeSource, MarshalSink, NoopResolver, RethCommitteeSource, RotateUpstream,
    MAX_UPSTREAM_FAULTS,
};
pub use cold_start_jump::{
    assert_l1_checkpoint, cold_start_jump, ElSync, RethElSync, JUMP_THRESHOLD,
};
pub use digest::Digest;
pub use dpos::{
    peek_consensus_archive_last_finalized, DposLayer, DposLayerConfig, DposLayerHandle,
    FollowerLayerConfig, FollowerRethHandle, PlaneMux, ResettableForward, RethHandle,
    SharedBeaconPlane, VoteBackupItem,
};
pub use epocher::OriginEpocher;
pub use feed_sink::FeedSink;
pub use order_block::{
    anchor_order_block, result_final_height, result_target, OrderBlock, ResultTarget, K,
};
pub use outer::{MarshalMailbox, OuterBuilder, OuterEngine, SoftEnterCommittees};
pub use timeouts::ConsensusTimeouts;

/// commonware journal replay buffer (per partition). Shared by the validator
/// marshal archives ([`outer`]) and the per-epoch Simplex engines ([`engine`]);
/// they MUST stay identical or storage replay desynchronizes between the two.
pub(crate) const REPLAY_BUFFER: std::num::NonZeroUsize =
    commonware_utils::NZUsize!(8 * 1024 * 1024);
/// commonware journal write buffer (per partition). Same shared-identity
/// constraint as [`REPLAY_BUFFER`].
pub(crate) const WRITE_BUFFER: std::num::NonZeroUsize = commonware_utils::NZUsize!(1024 * 1024);
