//! `FluentP2PConfig` — Fluent-side config struct + `to_commonware_config`
//! adapter that fills the rest from the must-be-identical hardcoded
//! constants in `crate::constants`.

use std::net::SocketAddr;

use commonware_cryptography::ed25519::PrivateKey;
use commonware_p2p::{
    authenticated::discovery::{Bootstrapper, Config as CommonwareConfig},
    Ingress,
};
use fluentbase_bls::{fluent_namespace, PeerPubkey};

use crate::constants;

/// Configuration for [`crate::FluentP2P::build`]. Includes only fields
/// that vary per-operator (safe-to-desync) or per-chain; every
/// network-invariant param comes from [`crate::constants`].
#[derive(Clone)]
pub struct FluentP2PConfig {
    /// Local Ed25519 keypair (Tier-3 peer key; on-chain
    /// `consensusKeys.peerPubkey`).
    pub crypto: PrivateKey,

    /// Chain ID — feeds `fluent_namespace(chain_id)`.
    pub chain_id: u64,

    /// Local bind socket (safe-to-desync — per-operator).
    pub listen: SocketAddr,

    /// What we tell peers to dial (safe-to-desync — per-operator).
    /// Socket-only in v1 (`ALLOW_DNS: false` in
    /// [`constants::ALLOW_DNS`]); the `Ingress` enum still has a `Dns`
    /// variant, but a misconfigured `Dns` here would be rejected by
    /// peers' verifiers (`InfoVerifier`).
    pub dialable: Ingress,

    /// Cold-start dial list (safe-to-desync — per-chain).
    pub bootstrappers: Vec<Bootstrapper<PeerPubkey>>,

    /// Allow RFC-1918 (private) ingress IPs for dev/local/test
    /// (CLI surface). Production must keep this `false`.
    pub allow_private_ips: bool,
}

impl FluentP2PConfig {
    /// Translate into a `commonware_p2p::Config<PrivateKey>` ready for
    /// `Network::new`. Use `Config::recommended` as the base (it sets
    /// `crypto` / `namespace` / `listen` / `dialable` / `bootstrappers` /
    /// `max_message_size` from its args, plus sensible defaults for the
    /// remaining fields), then override only the must-be-identical
    /// fields where our value differs from commonware's recommendation.
    pub fn to_commonware_config(&self) -> CommonwareConfig<PrivateKey> {
        CommonwareConfig {
            // Override commonware-recommended defaults to enforce
            // Fluent's network-wide invariants:
            allow_dns: constants::ALLOW_DNS, // recommended: true → ours: false
            allow_private_ips: self.allow_private_ips, // recommended: false → ours: per-operator
            max_peer_set_size: constants::MAX_PEER_SET_SIZE, // recommended: 2^16 → ours: 51
            // `tracked_peer_sets` not overridden: recommended default (4) is what we want.
            ..CommonwareConfig::recommended(
                self.crypto.clone(),
                &fluent_namespace(self.chain_id),
                self.listen,
                self.dialable.clone(),
                self.bootstrappers.clone(),
                constants::MAX_MESSAGE_SIZE,
            )
        }
    }
}
