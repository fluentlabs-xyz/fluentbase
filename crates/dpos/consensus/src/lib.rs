//! Fluent DPoS consensus: Commonware Simplex ⇄ Reth integration.
//!
//! Built as an **injection-style library** — collaborators are constructor
//! params, not awaited. Pinned against commonware `monorepo @ v2026.4.0`.
//!
//! Frozen leaves (collaborator-free):
//!   [`digest`] (block hash IS the Simplex digest — no separate hashing),
//!   [`elector_seed`] (per-epoch RoundRobin leader seed),
//!   [`timeouts`] (timeout family + commonware panic-invariant guard).
//!
//! Wiring layer ([`application`], [`block`], [`engine`], [`epoch_manager`],
//!   [`outer`], [`executor`], [`slasher`]): adapts upstream commonware
//!   Simplex behaviour to the Fluent Reth execution layer.

pub mod application;
pub mod block;
pub mod cert_follow;
pub mod digest;
pub mod dpos;
pub mod elector_seed;
pub mod engine;
pub mod epoch_manager;
pub mod epocher;
pub mod executor;
pub mod extra_data;
pub mod feed_sink;
pub mod outer;
pub mod reth_adapters;
pub mod scheme;
pub mod slasher;
pub mod timeouts;

pub use application::{BeaconEngineLike, FluentApp, PayloadAttrsBuilderLike, PayloadBuilderLike};
pub use block::Block;
pub use cert_follow::{
    CertFollowConfig, CertFollowHandle, CertFollowLayer, CertFollowRethHandle, CertUpstream,
    UpstreamFinalized,
};
pub use digest::Digest;
pub use dpos::{DposLayer, DposLayerConfig, DposLayerHandle, P2pParams, RethHandle};
pub use elector_seed::epoch_leader_seed;
pub use epocher::OriginEpocher;
pub use feed_sink::FeedSink;
pub use outer::{MarshalMailbox, OuterBuilder, OuterEngine};
pub use timeouts::ConsensusTimeouts;

/// commonware journal replay buffer (per partition). Shared by the validator
/// marshal archives ([`outer`]) and the per-epoch Simplex engines ([`engine`]);
/// they MUST stay identical or storage replay desynchronizes between the two.
pub(crate) const REPLAY_BUFFER: std::num::NonZeroUsize =
    commonware_utils::NZUsize!(8 * 1024 * 1024);
/// commonware journal write buffer (per partition). Same shared-identity
/// constraint as [`REPLAY_BUFFER`].
pub(crate) const WRITE_BUFFER: std::num::NonZeroUsize = commonware_utils::NZUsize!(1024 * 1024);
