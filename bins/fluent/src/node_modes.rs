//! Resolves the fluent-specific node modes (DPoS / cert-follow / sequencer /
//! staking) from parsed CLI args into one [`ResolvedModes`] struct, keeping the
//! construction out of `main`.

use std::time::Duration;

use alloy_primitives::Address;
use fluentbase_node::{
    consensus_rpc::FeedStateHandle,
    dpos::{CertFeed, CertInletCfg, DposConfig, FeedSink, FollowerCfg, NodeStackCfg},
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
    /// The ONE unified node-stack config (Phase 5): `Some` for `--dpos`
    /// (validator) OR `--cert-follow` (follower), `None` for a plain / trust-relay
    /// full node (the standalone `--sequencer-url` path stays `consensus_url` →
    /// `launch_consensus_node`, OUT OF SCOPE for `run_node_stack`).
    pub node_stack: Option<NodeStackCfg>,
    /// Cert-feed state handle shared into the `consensus` RPC closure. Set
    /// alongside `node_stack` when DPoS or cert-follow is enabled.
    pub cert_rpc_feed: Option<FeedStateHandle>,
    /// Pipeline 2 (sequencer→DPoS migration): parsed from `--dpos.staking-config`
    /// independent of `--dpos`. Sequencer and follower modes need non-zero addresses
    /// so the executor's `commitEpochCommittee` system call fires at epoch
    /// boundaries — required for prod migration past the first epoch boundary.
    pub staking_address: Address,
    pub chain_config_address: Address,
    /// `LivenessSlashing` address the executor's `processBitmap` system call
    /// targets. Lifted from the staking config (serde-defaulted to the canonical
    /// predeploy slot) so the whole cluster can be runtime-deployed. Unused when
    /// `staking_address` is zero (the system call is gated off).
    pub liveness_slashing_address: Address,
    /// Retained to build the activation-block reader for the sequencer's
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

    // If consensus URL is not specified, resolve default. Trust-follow keeps
    // using the FIRST entry of the (repeatable) --sequencer-url list (D3).
    if let Some(sequencer_url) = ext.sequencer_url.first() {
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

    // `--dpos` → unified node-stack, VALIDATOR overlay. `required_if_eq("dpos",
    // "true")` on the underlying clap fields guarantees the key/path `Option`s
    // are `Some` inside `DposConfig::from_args`. The cert-inlet is armed iff
    // `--dpos.follower-upstream` URLs are present (a rotated-out validator follows
    // the inlet-fed base); always `verify:true`.
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
        // feed actor (spawned inside the validator overlay).
        let feed_handle = FeedStateHandle::new(CERT_FEED_EVENT_CAP);
        let (feed_sink, feed_rx) = FeedSink::channel();
        modes.cert_rpc_feed = Some(feed_handle.clone());
        let upstreams = ext.dpos_cfg.dpos_follower_upstream.clone();
        let cert_inlet = if upstreams.is_empty() {
            None
        } else {
            Some(CertInletCfg { urls: upstreams })
        };
        modes.node_stack = Some(NodeStackCfg {
            is_validator: true,
            cert_inlet,
            validator: Some(DposConfig::from_args(
                &ext.dpos_cfg,
                Some(CertFeed {
                    sink: feed_sink,
                    rx: feed_rx,
                    handle: feed_handle,
                }),
            )),
            follower: None,
        });
    }

    // `--cert-follow` → unified node-stack, FOLLOWER overlay. Drives reth from
    // upstream-verified certs (the cert-inlet's SOLE producer, always
    // `verify:true`) instead of the trust-follow block relay. `requires_all`
    // (clap) guarantees both `--sequencer-url` and `--dpos.staking-config` are
    // present. Clear `consensus_url` so `launch_consensus_node` does not also run.
    if ext.cert_follow {
        assert!(
            !ext.sequencer_url.is_empty(),
            "requires_all guarantees --sequencer-url"
        );
        // Serving side (D4): the follower serves the same `consensus` WS
        // namespace as validators, window-backed. The handle is shared into
        // the RPC closure exactly like the DPoS branch.
        let feed_handle = FeedStateHandle::new(CERT_FEED_EVENT_CAP);
        modes.cert_rpc_feed = Some(feed_handle.clone());
        let l1 = ext.cert_follow_l1_rpc_url.as_ref().map(|url| {
            fluentbase_node::cert_follow::l1::L1CheckpointConfig {
                rpc_url: url.clone(),
                rollup_address: ext
                    .cert_follow_l1_rollup_address
                    .expect("clap `requires` pairs the two L1 flags"),
            }
        });
        modes.node_stack = Some(NodeStackCfg {
            is_validator: false,
            cert_inlet: Some(CertInletCfg {
                urls: ext.sequencer_url.clone(),
            }),
            validator: None,
            follower: Some(FollowerCfg {
                feed: Some(feed_handle),
                l1,
                staking_config_path: ext
                    .dpos_cfg
                    .dpos_staking_config
                    .clone()
                    .expect("requires_all guarantees --dpos.staking-config"),
            }),
        });
        modes.consensus_url = None;
    }

    // sequencer→DPoS migration: parse `--dpos.staking-config` independently of
    // `--dpos`. Sequencer and follower modes also need non-zero `staking_address` /
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
    // for plain / sequencer-migration nodes, handled above.)
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
            modes.liveness_slashing_address = parsed.liveness_slashing_address;
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
