use fluentbase_sdk::U256;

use crate::consts::BALANCE_COMPACT_PRECISION;

pub fn compact_balance(amount: U256) -> Option<U256> {
    if amount % BALANCE_COMPACT_PRECISION != U256::ZERO {
        return None;
    }
    Some(amount / BALANCE_COMPACT_PRECISION)
}

pub fn epoch_at_block(block_number: u64, activation_block: u64, interval: u64) -> Option<u64> {
    if interval == 0 {
        return None;
    }
    if block_number < activation_block {
        return Some(0);
    }
    Some((block_number - activation_block) / interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_balance_rejects_precision_dust() {
        assert_eq!(
            compact_balance(U256::from(10_000_000_000u64)),
            Some(U256::ONE)
        );
        assert_eq!(compact_balance(U256::from(10_000_000_001u64)), None);
    }

    #[test]
    fn epoch_is_rebased_and_clamped_before_activation() {
        assert_eq!(epoch_at_block(99, 100, 20), Some(0));
        assert_eq!(epoch_at_block(100, 100, 20), Some(0));
        assert_eq!(epoch_at_block(139, 100, 20), Some(1));
        assert_eq!(epoch_at_block(140, 100, 20), Some(2));
        assert_eq!(epoch_at_block(140, 100, 0), None);
    }
}
