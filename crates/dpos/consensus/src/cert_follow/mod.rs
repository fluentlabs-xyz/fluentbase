//! Cert-follow transport + geometry seam.
//!
//! After the unified-node-mode collapse the cert-follow *engine* no longer
//! exists as a separate writer: a non-validator follower is a near-planeless
//! [`crate::dpos::DposLayer::launch_follower`] (inlet + executor, the executor
//! being the sole reth writer), and an upstream-configured validator runs the
//! cert-inlet as a second producer into its own marshal. What survives here is
//! only the transport-agnostic seam both paths share:
//!
//!   - [`CertUpstream`] / [`UpstreamFinalized`] — the by-height pull + live
//!     finalized-cert stream the node's WS actor implements (the inlet's sole
//!     producer; the cold-start JUMP's `get_latest` source).
//!   - [`read_geometry`] — the codeless-tolerant epoch-geometry read that
//!     discriminates a RESTART datadir (geometry readable from local state) from
//!     a FRESH datadir (geometry readable only AFTER EL-sync). Used by the
//!     follower cold-start in [`crate::dpos`].
//!
//! The per-epoch BLS verifier read ([`crate::cert_inlet::RethCommitteeSource`])
//! and the EL-sync JUMP ([`crate::cold_start_jump::RethElSync`]) live in their
//! own modules.

mod upstream;

use alloy_consensus::Header;
use alloy_primitives::B256;
use eyre::ensure;
use fluentbase_staking_reader::RethStakingStateReader;
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::ConfigureEvm;
use reth_storage_api::{HeaderProvider, StateProviderFactory};
pub use upstream::{CertUpstream, UpstreamFinalized};

/// Codeless-tolerant epoch-geometry read: `None` when `ChainConfig` is not
/// deployed (or DPoS not yet scheduled) at `at` — the launch discriminator
/// between "restart datadir / genesis-baked devnet" and "fresh datadir on a
/// runtime-deployed chain", where geometry is only readable AFTER EL-sync.
pub fn read_geometry<Provider, EvmConfig>(
    reader: &RethStakingStateReader<Provider, EvmConfig>,
    at: B256,
) -> eyre::Result<Option<(u64, u32)>>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    match reader.scheduled_dpos_activation(at)? {
        None => Ok(None),
        Some(activation) => {
            let interval = reader.epoch_block_interval(at)?;
            ensure!(interval > 0, "epoch_block_interval must be > 0");
            Ok(Some((activation, interval)))
        }
    }
}
