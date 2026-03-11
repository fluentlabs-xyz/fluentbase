use crate::chainspec::{FLUENT_DEVNET, FLUENT_MAINNET, FLUENT_TESTNET};
use reth_chainspec::Chain;
use reth_network_peers::TrustedPeer;

pub const DEVNET_TRUSTED_PEERS: [&'static str; 1] = [
    "enode://1fde05d9bd808bbdb13f1db7ed74d4ca33d2155ad15efcb8c082013c9d61723478cdb4de2c2a167648daa9bd2f6006463ed6cb28ff003a056547d116cac86df2@104.248.141.60:30303",
];

pub const TESTNET_TRUSTED_PEERS: [&'static str; 2] = [
    "enode://730f7de363021325f278f79d49a46d9379198293307d937f4935b569effba7733a8836cd950fc58e3801b3db4604f755fa680cbed2e7d8d869688eb554d8fafc@64.226.97.106:30303",
    "enode://a82f7b87d4c04b8379797ead0d60a5f736835df0d10a120fa75405235d9146e263555d9b908788a3b080fbee108d21d4516716d35801f5f526aab7b5bc46ecf3@68.183.211.71:30303",
];

pub const MAINNET_TRUSTED_PEERS: [&'static str; 0] = [];

pub fn resolve_default_trusted_peers(chain: Chain) -> Vec<TrustedPeer> {
    let trusted_peers = if chain == FLUENT_DEVNET.chain {
        &DEVNET_TRUSTED_PEERS[..]
    } else if chain == FLUENT_TESTNET.chain {
        &TESTNET_TRUSTED_PEERS[..]
    } else if chain == FLUENT_MAINNET.chain {
        &[]
    } else {
        &[]
    };
    trusted_peers.iter().map(|s| s.parse().unwrap()).collect()
}

pub const DEVNET_CONSENSUS_URL: &'static str = "wss://rpc.devnet.fluent.xyz";
pub const TESTNET_CONSENSUS_URL: &'static str = "wss://rpc.testnet.fluent.xyz";

pub fn resolve_default_consensus_url(chain: Chain) -> Option<String> {
    if chain == FLUENT_DEVNET.chain {
        Some(DEVNET_CONSENSUS_URL.to_string())
    } else if chain == FLUENT_TESTNET.chain {
        Some(TESTNET_CONSENSUS_URL.to_string())
    } else if chain == FLUENT_MAINNET.chain {
        None
    } else {
        None
    }
}
