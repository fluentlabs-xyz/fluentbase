//! Fluent chain rules that differ from Ethereum, and the historical Testnet rules. See
//! `docs/release-upgrade.md` for the canonical evidence of the latter.

use crate::U256;

/// The highest gas limit a transaction may declare.
///
/// Fluent does not apply the fixed `2^24` of EIP-7825, which WASM execution can legitimately
/// exceed, but it does cap transactions: this is the highest block gas limit any Fluent network
/// has had (mainnet genesis), so no historical transaction is above it and it only binds once a
/// block gas limit rises past it. Consensus-critical: a block carrying a larger transaction is
/// invalid, and the transaction pool rejects one. See `docs/04-gas-and-fuel.md`.
pub const TX_GAS_LIMIT_CAP: u64 = 100_000_000;

pub const FLUENT_TESTNET_CHAIN_ID: u64 = 0x5202;

/// First block after the last canonical non-fee-manager beneficiary.
pub const TESTNET_FEE_MANAGER_BLOCK: u64 = 21_755_352;

/// First full-fee transaction block at the historical transition. Blocks 21,781,415 and
/// 21,781,416 are empty, so choosing this boundary does not affect their fee credit.
pub const TESTNET_FULL_FEES_BLOCK: u64 = 21_781_417;

/// Historical Testnet transactions credited only the priority fee. Evaluate this from the
/// current block, including when an EVM instance is reused across the transition.
pub fn testnet_burns_base_fee(chain_id: u64, block_number: U256) -> bool {
    chain_id == FLUENT_TESTNET_CHAIN_ID && block_number < U256::from(TESTNET_FULL_FEES_BLOCK)
}
