//! The `consensus` RPC namespace: serves finality certificates to cert-followers
//! (`getFinalization` + `getLatest` + `subscribe`), mirroring tempo's `rpc/consensus`.
//!
//! v1 is finalized-only (F2=b). Wiring: the consensus `FeedSink` forwards finalized
//! heights → [`feed_actor::FeedActor`] builds [`CertifiedBlock`]s into
//! [`state::FeedStateHandle`] → [`server::ConsensusRpc`] serves them over jsonrpsee
//! (registered via reth `extend_rpc_modules`).

pub mod feed_actor;
pub mod server;
pub mod state;
pub mod types;

pub(crate) use feed_actor::now_ms;
pub use server::{ConsensusApiClient, ConsensusApiServer, ConsensusRpc};
pub use state::FeedStateHandle;
pub use types::{ConsensusState, Event, Query};
