//! `FluentP2PConfig` — Fluent-side config struct plus the `into_commonware_config`
//! adapter; every network-invariant parameter comes from `crate::constants`.

use std::{net::SocketAddr, time::Duration};

use commonware_cryptography::ed25519::PrivateKey;
use commonware_p2p::{
    authenticated::discovery::{Bootstrapper, Config as CommonwareConfig},
    Ingress,
};
use fluentbase_bls::{fluent_namespace, PeerPubkey};

use crate::constants;

/// Configuration for [`crate::FluentP2P::build`]: only fields that vary
/// per-operator (safe-to-desync) or per-chain.
///
/// Not `Clone`: `into_commonware_config` consumes it by value, and it holds the
/// secret ed25519 `crypto` key.
pub struct FluentP2PConfig {
    /// Local Ed25519 keypair (Tier-3 peer key; on-chain `consensusKeys.peerPubkey`).
    pub crypto: PrivateKey,

    /// Chain ID — feeds `fluent_namespace(chain_id)`.
    pub chain_id: u64,

    /// Local bind socket.
    pub listen: SocketAddr,

    /// What we tell peers to dial: an `Ingress::Socket` or an `Ingress::Dns`
    /// hostname, which peers resolve at dial time and re-filter against
    /// `allow_private_ips` post-resolution. Whether DNS is accepted is a
    /// network-wide switch, not a per-operator one — see [`constants::ALLOW_DNS`].
    pub dialable: Ingress,

    /// Cold-start dial list.
    pub bootstrappers: Vec<Bootstrapper<PeerPubkey>>,
}

impl FluentP2PConfig {
    /// Translate into a `commonware_p2p::Config<PrivateKey>` for `Network::new`:
    /// start from `Config::recommended` (which sets `crypto` / `namespace` /
    /// `listen` / `dialable` / `bootstrappers` / `max_message_size` plus sensible
    /// defaults) and override only the must-be-identical fields where our value
    /// differs.
    pub fn into_commonware_config(self) -> CommonwareConfig<PrivateKey> {
        // Restart-rejoin latency vs. production rate-limiting. commonware's
        // `recommended()` cooldown of 60s and gossip frequency of 50s are
        // anti-thrash limits, but a restarted validator's first outbound dial is
        // dropped by peers holding a stale half-open session from its prior
        // incarnation, leaving it isolated for a minute with no finalized gossip.
        // Deployed public networks keep that cadence; localnet and ad-hoc local
        // chains re-peer in seconds. Keyed on `chain_id`, which all nodes of a
        // network share, so the values stay identical network-wide.
        const FLUENT_DEVNET_CHAIN_ID: u64 = 0x5201;
        const FLUENT_TESTNET_CHAIN_ID: u64 = 0x5202;
        const FLUENT_MAINNET_CHAIN_ID: u64 = 25363;
        let deployed_public_network = matches!(
            self.chain_id,
            FLUENT_DEVNET_CHAIN_ID | FLUENT_TESTNET_CHAIN_ID | FLUENT_MAINNET_CHAIN_ID
        );
        let (peer_connection_cooldown, gossip_bit_vec_frequency) = if deployed_public_network {
            (Duration::from_secs(60), Duration::from_secs(50))
        } else {
            (Duration::from_secs(1), Duration::from_secs(5))
        };
        // Deployed public networks join via the Fluent-curated public-IP
        // bootstrappers JSON, so RFC-1918 ingress is never legitimate there, while
        // local/ad-hoc chains need it. Derived from the same predicate so a
        // deployed network cannot accept private ingress via operator misconfig.
        let allow_private_ips = !deployed_public_network;
        CommonwareConfig {
            allow_dns: constants::ALLOW_DNS,
            allow_private_ips,
            max_peer_set_size: constants::MAX_REGISTRY_PEER_SET,
            peer_connection_cooldown,
            gossip_bit_vec_frequency,
            // `tracked_peer_sets` not overridden: recommended default (4) is what we want.
            ..CommonwareConfig::recommended(
                self.crypto,
                &fluent_namespace(self.chain_id),
                self.listen,
                self.dialable,
                self.bootstrappers,
                constants::MAX_MESSAGE_SIZE,
            )
        }
    }
}
