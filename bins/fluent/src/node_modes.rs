//! Resolves the fluent-specific node modes (DPoS / cert-follow / sequencer /
//! staking) from parsed CLI args into one [`ResolvedModes`] struct, keeping the
//! construction out of `main`.

use std::time::Duration;

use alloy_primitives::Address;
use fluentbase_node::{
    cert_follow::CertFollowerConfig,
    consensus_rpc::FeedStateHandle,
    dpos::{CertFeed, DposConfig, FeedSink},
    trusted_peers::resolve_default_consensus_url,
};
use reth_chainspec::Chain;

use crate::{FluentNodeArgs, CERT_FEED_EVENT_CAP};

/// Fluent-mode configuration resolved from `--validator` / `--dpos` /
/// `--cert-follow` / `--sequencer-url` / `--dpos.staking-config`. The `Default`
/// values are exactly the non-`Node`-command fallbacks `main` relies on.
#[derive(Default)]
pub(crate) struct ResolvedModes {
    pub consensus_url: Option<String>,
    pub block_producer: Option<Duration>,
    pub dpos_config: Option<DposConfig>,
    pub cert_follow_config: Option<CertFollowerConfig>,
    /// Cert-feed state handle shared into the `consensus` RPC closure. Set
    /// alongside `dpos_config` when DPoS is enabled.
    pub cert_rpc_feed: Option<FeedStateHandle>,
    /// Pipeline 2 (Tempo→DPoS migration): parsed from `--dpos.staking-config`
    /// independent of `--dpos`. Tempo and follower modes need non-zero addresses
    /// so the executor's `commitEpochCommittee` system call fires at epoch
    /// boundaries — required for prod migration past the first epoch boundary.
    pub staking_address: Address,
    pub chain_config_address: Address,
    /// Retained to build the activation-block reader for the Tempo sequencer's
    /// production gate (clean-halt at `dposActivationBlock`).
    pub staking_reader_cfg: Option<fluentbase_staking_reader::reader::StakingReaderConfig>,
}

/// Build [`ResolvedModes`] from the parsed node args. Mirrors the prior inline
/// `if let Commands::Node(node)` body in `main`. `std::process::exit(1)` on a
/// partial/invalid `--dpos.staking-config` is preserved (fail-loud at load).
pub(crate) fn resolve_node_modes(
    ext: &FluentNodeArgs,
    chain: Chain,
    debug_consensus_url: Option<&str>,
) -> ResolvedModes {
    let mut modes = ResolvedModes::default();

    // If consensus URL is not specified, resolve default
    if let Some(sequencer_url) = &ext.sequencer_url {
        modes.consensus_url = Some(sequencer_url.clone());
    } else if let Some(debug_consensus_url) = debug_consensus_url {
        modes.consensus_url = Some(debug_consensus_url.to_owned());
    } else {
        modes.consensus_url = resolve_default_consensus_url(chain);
    }

    // If validator mode is enabled then specify block production time
    if ext.validator {
        modes.block_producer = Some(ext.validator_block_time);
    }

    // If DPoS mode is enabled, build the DposConfig from required args.
    // `required_if_eq("dpos", "true")` on the underlying clap fields
    // guarantees the `Option`s are `Some` here.
    if ext.dpos {
        // DPoS drives reth itself via its own marshal/executor. Clear any default
        // or debug consensus URL so `launch_consensus_node` (the unverified
        // trust-follow block relay) does not ALSO run a second engine driver
        // against reth — on a named chain `resolve_default_consensus_url` would
        // otherwise point a validator at the public sequencer WSS and fight the
        // DPoS executor for reth's head (audit P2-1).
        modes.consensus_url = None;
        // Cert-feed wiring: the FeedSink is the marshal's 2nd Reporter; the
        // handle is shared with the `consensus` RPC; the receiver drives the
        // feed actor (spawned inside `run_dpos_stack`).
        let feed_handle = FeedStateHandle::new(CERT_FEED_EVENT_CAP);
        let (feed_sink, feed_rx) = FeedSink::channel();
        modes.cert_rpc_feed = Some(feed_handle.clone());
        modes.dpos_config = Some(DposConfig::from_args(
            &ext.dpos_cfg,
            Some(CertFeed {
                sink: feed_sink,
                rx: feed_rx,
                handle: feed_handle,
            }),
        ));
    }

    // Cert-follower mode: drive reth from upstream-verified certs instead of
    // the trust-follow block relay. `requires_all` (clap) guarantees both
    // `--sequencer-url` and `--dpos.staking-config` are present. Clear
    // `consensus_url` so `launch_consensus_node` (the trust path) does not
    // also run — the follower drives reth itself.
    if ext.cert_follow {
        modes.cert_follow_config = Some(CertFollowerConfig {
            sequencer_url: ext
                .sequencer_url
                .clone()
                .expect("requires_all guarantees --sequencer-url"),
            staking_config_path: ext
                .dpos_cfg
                .dpos_staking_config
                .clone()
                .expect("requires_all guarantees --dpos.staking-config"),
        });
        modes.consensus_url = None;
    }

    // Tempo→DPoS migration: parse `--dpos.staking-config` independently of
    // `--dpos`. Tempo and follower modes also need non-zero `staking_address` /
    // `chain_config_address` so the existing `commitEpochCommittee` system call
    // in `FluentBlockExecutor::apply_pre_execution_changes`
    // ([crates/node/src/evm.rs:848](../../../crates/node/src/evm.rs#L848)) fires
    // at epoch boundaries. Without this, post-swap DPoS validators reading
    // `epoch_committee_snapshot(epoch_k, finalized_hash)` would see an empty
    // committee for any epoch > 0.
    if let Some(path) = &ext.dpos_cfg.dpos_staking_config {
        apply_staking_config(&mut modes, path, "--dpos.staking-config");
    }

    // DPoS / cert-follow REQUIRE non-zero staking addresses: both modes read the
    // committee from the staking contract at runtime (`epoch_committee_snapshot`),
    // which against address zero fails post-launch with an opaque decode error and
    // exits 0 (audit P2-20). Fail loud at load instead. (Both-zero is legal only
    // for plain / tempo-migration nodes, handled above.)
    if (ext.dpos || ext.cert_follow)
        && (modes.staking_address.is_zero() || modes.chain_config_address.is_zero())
    {
        eprintln!(
            "--dpos / --cert-follow require a --dpos.staking-config with non-zero \
             staking_address and chain_config_address (got {} / {})",
            modes.staking_address, modes.chain_config_address
        );
        std::process::exit(1);
    }

    modes
}

/// Modes for non-`node` subcommands (`import` / `stage` / `re-execute`).
///
/// `--dpos.staking-config` is a `node`-subcommand Ext flag, so those commands
/// default to zero staking addresses — but post-migration DPoS blocks re-execute
/// through the same `FluentBlockExecutor` whose `commitEpochCommittee` /
/// `processBitmap` system calls WRITE canonical state. With the addresses zero
/// the DPoS gate is off and the recomputed state root diverges (import fails at
/// the Merkle stage; re-execute falsely reports corruption). Source the addresses
/// from the env var so re-execution matches the live chain (audit P2-16).
pub(crate) fn resolve_non_node_modes() -> ResolvedModes {
    let mut modes = ResolvedModes::default();
    if let Ok(path) = std::env::var("FLUENT_DPOS_STAKING_CONFIG") {
        apply_staking_config(
            &mut modes,
            std::path::Path::new(&path),
            "FLUENT_DPOS_STAKING_CONFIG",
        );
    }
    modes
}

/// Parse a staking-reader config and wire `staking_address` /
/// `chain_config_address` / `staking_reader_cfg` into `modes`, fail-loud at load
/// on a parse error or a partial (one-zero) config. `source` labels the input
/// (CLI flag vs env var) in diagnostics. Shared by the node and non-node paths so
/// the both-or-neither guard can't drift between them.
fn apply_staking_config(modes: &mut ResolvedModes, path: &std::path::Path, source: &str) {
    match fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(path) {
        Ok(parsed) => {
            modes.staking_address = parsed.staking_address;
            modes.chain_config_address = parsed.chain_config_address;
            modes.staking_reader_cfg = Some(parsed);
            // Both-or-neither: the committee-commit gate (evm.rs) needs BOTH
            // addresses non-zero. A one-zero typo would silently downgrade the
            // node to plain Ethereum (or, on import/re-execute, diverge the state
            // root) with no error — fail loud at load instead.
            if modes.staking_address.is_zero() != modes.chain_config_address.is_zero() {
                eprintln!(
                    "{source} partial config: staking_address ({}) and chain_config_address \
                     ({}) must be BOTH zero (non-DPoS) or BOTH non-zero (DPoS)",
                    modes.staking_address, modes.chain_config_address
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("failed parsing {source} at {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}
