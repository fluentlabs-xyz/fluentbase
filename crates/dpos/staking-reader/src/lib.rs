//! In-process reader for the Fluent staking system contract.
//!
//! # Invariants
//!
//! - **In-process only.** Reth state via `StateProviderFactory` + `ConfigureEvm`,
//!   no JSON-RPC and no network hop.
//! - **Hash-keyed.** Every read names an explicit block hash, never "latest", so it
//!   is deterministic and reorg-safe.
//! - **Order verbatim.** `getEpochCommittee` order (frozen ascending peer pubkey)
//!   is surfaced exactly as returned; this crate never sorts.
//! - **Single key decoder.** BLS/ed25519 keys go through the subgroup-checked
//!   `fluentbase-bls` decoders; no second parser.
//! - **Epoch is local math.** The relative epoch comes from `epoch_at_block` over
//!   the activation height and interval, never from a per-epoch `currentEpoch()` call.
//!
//! # Wiring
//!
//! ```ignore
//! let reader = RethStakingStateReader::new(
//!     handle.node.provider.clone(),   // StateProviderFactory + HeaderProvider
//!     handle.node.evm_config.clone(), // FluentEvmConfig: ConfigureEvm
//!     StakingReaderConfig::from_json_path(&cfg_path)?,
//! );
//! ```
//! Reads are synchronous and hit the reth database directly, so call them from a
//! `spawn_blocking` context.

pub mod epoch_transition;
pub mod error;
pub mod reader;

pub use epoch_transition::{EpochTransition, PeerSetSink, TrackedPeers, TransitionOutcome};
pub use error::{ReadClass, ReadError};
pub use reader::{classify_transient_provider_error, RethStakingStateReader, StakingStateRead};
