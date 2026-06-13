//! `FluentP2PConfig` — Fluent-side config struct + `to_commonware_config`
//! adapter that fills the rest from the must-be-identical hardcoded
//! constants in `crate::constants`.

use std::{net::SocketAddr, time::Duration};

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
}

impl FluentP2PConfig {
    /// Translate into a `commonware_p2p::Config<PrivateKey>` ready for
    /// `Network::new`. Use `Config::recommended` as the base (it sets
    /// `crypto` / `namespace` / `listen` / `dialable` / `bootstrappers` /
    /// `max_message_size` from its args, plus sensible defaults for the
    /// remaining fields), then override only the must-be-identical
    /// fields where our value differs from commonware's recommendation.
    pub fn to_commonware_config(&self) -> CommonwareConfig<PrivateKey> {
        // Restart-rejoin latency vs. production rate-limiting, selected per network.
        // commonware's `recommended()` (and tempo's production profile) set
        // `peer_connection_cooldown = 60s` / `gossip_bit_vec_frequency = 50s` as a
        // deliberate anti-thrash/DoS rate-limit. But after a validator restart its
        // first outbound dial is dropped by the peer (stale half-open session from the
        // prior incarnation), and the 60s cooldown then blocks reconnection until the
        // peer re-dials inbound — leaving a restarted node isolated ~60-83s (no
        // finalized gossip → marshal can't backfill → simplex voter spins nullify on
        // its last view). Mirror tempo's `use_local_defaults` split, keyed on the
        // network: the deployed public networks (devnet/testnet/mainnet) keep the
        // conservative cadence; localnet and ad-hoc local chains (the devnet smoke,
        // custom genesis) re-peer in seconds. All nodes on a network share its
        // `chain_id`, so the selected values stay identical network-wide (the
        // `gossip_bit_vec_frequency` G11 invariant in `constants`). Chain IDs mirror
        // `crates/node/src/chainspec.rs` (canonical source).
        const FLUENT_DEVNET_CHAIN_ID: u64 = 0x5201;
        const FLUENT_TESTNET_CHAIN_ID: u64 = 0x5202;
        const FLUENT_MAINNET_CHAIN_ID: u64 = 25363;
        let deployed_public_network = matches!(
            self.chain_id,
            FLUENT_DEVNET_CHAIN_ID | FLUENT_TESTNET_CHAIN_ID | FLUENT_MAINNET_CHAIN_ID
        );
        let (peer_connection_cooldown, gossip_bit_vec_frequency) = if deployed_public_network {
            (Duration::from_secs(60), Duration::from_secs(50)) // commonware/tempo production default
        } else {
            (Duration::from_secs(1), Duration::from_secs(5)) // localnet / local dev: re-peer in seconds
        };
        // Deployed public networks join via the Fluent-curated public-IP
        // bootstrappers JSON, so RFC-1918 ingress is never legitimate there;
        // local/ad-hoc chains (127.0.0.0/8, RFC-1918) need it. Derived from
        // the same predicate so a deployed network can't accept private
        // ingress via operator misconfig.
        let allow_private_ips = !deployed_public_network;
        CommonwareConfig {
            // Override commonware-recommended defaults to enforce
            // Fluent's network-wide invariants:
            allow_dns: constants::ALLOW_DNS, // recommended: true → ours: false
            allow_private_ips, // recommended: false → ours: network-derived (deployed → false)
            max_peer_set_size: constants::MAX_REGISTRY_PEER_SET, // tracker feed = registry ∪ committee
            peer_connection_cooldown,
            gossip_bit_vec_frequency,
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
