//! Historical Fluent Testnet rules. See `docs/release-upgrade.md` for the canonical evidence.

use crate::U256;

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
