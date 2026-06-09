//! In-process reader + cache for the Fluent staking system contract.
//!
//! Foundational DPoS read layer: calls the staking and `ChainConfig` system
//! contracts' `view` functions from the node's own reth state and decodes
//! them into hybrid Rust types, plus a bounded validator-set cache.
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
//!   sorts. (The stake-DESC `getValidatorsWithKeys` candidate read was
//!   removed; the cache stores the frozen committee, not candidates.)
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
//!
//! # Validator-set cache wiring
//!
//! [`ValidatorSetCache`] needs a commonware-runtime `Storage` context that a
//! bare reth node does not have. The node builds a
//! `commonware_runtime::tokio` context rooted at the node datadir, applies
//! the metrics label, and constructs the cache:
//!
//! ```ignore
//! let cfg = commonware_runtime::tokio::Config::default()
//!     .with_storage_directory(reth_datadir.join("staking-reader"));
//! commonware_runtime::tokio::Runner::new(cfg).start(|ctx| async move {
//!     let cache = ValidatorSetCache::init(
//!         ctx.with_label("staking-reader-cache")).await?;
//! });
//! ```
//! `init` does **not** add its own metrics label (the node supplies it; a
//! same-namespace re-init would double-register and panic). The cache is
//! the durable epoch-indexed archive only: the
//! speculative in-mem hot tier was removed — under finality-gated apply
//! there is no tentative window and no sync `get_hot` consumer.
//! `persist_final`/`get`/`contains`/`prune` are driven by `epoch_transition`.

pub mod cache;
pub mod epoch_transition;
pub mod error;
pub mod reader;

pub use cache::ValidatorSetCache;
pub use epoch_transition::{EpochTransition, PeerSetSink, TransitionOutcome};
pub use error::ReadError;
pub use reader::{RethStakingStateReader, StakingStateRead};
