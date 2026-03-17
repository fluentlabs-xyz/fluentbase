use crate::chainspec::{FLUENT_DEVNET_CHAIN_ID, FLUENT_MAINNET_CHAIN_ID, FLUENT_TESTNET_CHAIN_ID};
use reth_chainspec::Chain;
use reth_network_peers::TrustedPeer;

pub const DEVNET_TRUSTED_PEERS: [&str; 1] = [
    "enode://1fde05d9bd808bbdb13f1db7ed74d4ca33d2155ad15efcb8c082013c9d61723478cdb4de2c2a167648daa9bd2f6006463ed6cb28ff003a056547d116cac86df2@104.248.141.60:30303",
];

pub const TESTNET_TRUSTED_PEERS: [&str; 2] = [
    "enode://730f7de363021325f278f79d49a46d9379198293307d937f4935b569effba7733a8836cd950fc58e3801b3db4604f755fa680cbed2e7d8d869688eb554d8fafc@64.226.97.106:30303",
    "enode://a82f7b87d4c04b8379797ead0d60a5f736835df0d10a120fa75405235d9146e263555d9b908788a3b080fbee108d21d4516716d35801f5f526aab7b5bc46ecf3@68.183.211.71:30303",
];

pub const MAINNET_TRUSTED_PEERS: [&str; 3] = [
    "enode://febc3d382a427ad8e592c5daab04c0f5656f275dac319969e83d2631da77386762560cfb76ad7ab3b1945dffdece4f3f208b8c7708d21470ba7b1d8afd0fb3f2@64.225.109.83:30303",
    "enode://b6bad967e0bee436ac94a9a7e30fd718cc0b2a0064062579d4c175c24746229169fc828029023c8a42c526b1ad6282128574ad903382ad43423f8fbfde990def@159.223.19.57:30303",
    "enode://5caf524726376e39f05280cfa2172d2eb46e24daa26b4e7a97e0d19d46e2cf01775301e3b026bd5a491ad39048bd2ac4c26315726a45f766d7372ffd6de3afa4@64.226.113.161:30303",
];

#[allow(clippy::if_same_then_else)]
pub fn resolve_default_trusted_peers(chain: Chain) -> Vec<TrustedPeer> {
    let trusted_peers = if chain == Chain::from(FLUENT_DEVNET_CHAIN_ID) {
        &DEVNET_TRUSTED_PEERS[..]
    } else if chain == Chain::from(FLUENT_TESTNET_CHAIN_ID) {
        &TESTNET_TRUSTED_PEERS[..]
    } else if chain == Chain::from(FLUENT_MAINNET_CHAIN_ID) {
        &MAINNET_TRUSTED_PEERS[..]
    } else {
        &[]
    };
    trusted_peers.iter().map(|s| s.parse().unwrap()).collect()
}

pub const DEVNET_SEQUENCER_URL: &str = "wss://rpc.devnet.fluent.xyz";
pub const TESTNET_SEQUENCER_URL: &str = "wss://rpc.testnet.fluent.xyz";
pub const MAINNET_SEQUENCER_URL: &str = "wss://rpc.fluent.xyz";

#[allow(clippy::if_same_then_else)]
pub fn resolve_default_consensus_url(chain: Chain) -> Option<String> {
    if chain == Chain::from(FLUENT_DEVNET_CHAIN_ID) {
        Some(DEVNET_SEQUENCER_URL.to_string())
    } else if chain == Chain::from(FLUENT_TESTNET_CHAIN_ID) {
        Some(TESTNET_SEQUENCER_URL.to_string())
    } else if chain == Chain::from(FLUENT_MAINNET_CHAIN_ID) {
        Some(MAINNET_SEQUENCER_URL.to_string())
    } else {
        None
    }
}
