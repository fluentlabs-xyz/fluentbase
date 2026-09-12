//! In-process reader for the Fluent staking system contract.
//!
//! Foundational DPoS read layer: calls the staking and `ChainConfig` system
//! contracts' `view` functions from the node's own reth state and decodes
//! them into hybrid Rust types.
//!
//! # Invariants
//!
//! - **In-process only.** Production reads go through the node's own reth
//!   state (`StateProviderFactory` + `ConfigureEvm`), not JSON-RPC. No L1,
//!   no RPC client, no network hop.
//! - **Hash-keyed.** Every read is keyed by an explicit block hash —
//!   deterministic and reorg-safe. The reader never picks "latest".
//! - **Order verbatim.** `getEpochCommittee` (frozen ascending-peerPubkey)
//!   order is surfaced exactly as the contract returns it; this crate never
//!   sorts. (The stake-DESC `getValidatorsWithKeys` candidate read was removed.)
//! - **Single key decoder.** BLS/ed25519 keys are decoded via the
//!   subgroup-checked `fluentbase-bls` decoders — the same path the
//!   consensus layer trusts; no second parser.
//! - **Epoch is local math.** `epoch = block.number / epochBlockInterval`;
//!   no per-epoch `currentEpoch()` call.
//! - **Seam added on demand.** `RethStakingStateReader` is the concrete reader;
//!   the `StakingStateRead` trait over it (in [`reader`]) was added once real
//!   consumers appeared (the epoch-boundary orchestrator, the slasher,
//!   `OuterEngine`) — each stays generic over the reader and mockable in tests.
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
//! Reads are synchronous and hash-keyed; call them from a `spawn_blocking`
//! context (reth state reads are blocking DB reads).

pub mod epoch_transition;
pub mod error;
pub mod reader;

pub use epoch_transition::{EpochTransition, PeerSetSink, TrackedPeers, TransitionOutcome};
pub use error::{ReadClass, ReadError};
pub use reader::{classify_transient_provider_error, RethStakingStateReader, StakingStateRead};
