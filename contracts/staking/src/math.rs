//! Pure arithmetic shared by staking state transitions.

use fluentbase_sdk::{staking_protocol, Uint, U256};

use crate::consts::BALANCE_COMPACT_PRECISION;

pub type U96 = Uint<96, 2>;
/// The compact stake unit. Its width is shared with the node, which rejects any
/// weight at or above `2^COMPACT_STAKE_BITS` before the leader elector sees it.
pub type U112 = Uint<{ staking_protocol::COMPACT_STAKE_BITS }, 2>;

/// Convert a full-precision BLEND balance into the compact `uint112` unit.
pub fn compact_balance(amount: U256) -> Option<U112> {
    if amount % BALANCE_COMPACT_PRECISION != U256::ZERO {
        return None;
    }
    U112::checked_from_limbs_slice((amount / BALANCE_COMPACT_PRECISION).as_limbs())
}

/// Restore a compact staking balance to full-precision BLEND wei.
pub fn expand_balance(amount: U112) -> U256 {
    U256::from(amount) * BALANCE_COMPACT_PRECISION
}

/// Narrow an epoch reward to the Solidity `uint96` ledger width.
pub fn narrow_reward(amount: U256) -> Option<U96> {
    U96::checked_from_limbs_slice(amount.as_limbs())
}

/// Simplex fault tolerance `f = ⌊(n−1)/3⌋`, re-exported from the shared crate.
///
/// One definition with the off-chain consensus, so the correlation guard and the
/// concurrent-exclusion ceiling cannot disagree with the budget commonware
/// actually applies to a quorum. A node-side test compares the shared function
/// against `N3f1::max_faults` from the pinned commonware checkout.
pub use staking_protocol::fault_tolerance;

/// Map a block to its activation-relative epoch, clamping pre-activation blocks.
///
/// The formula and the pre-activation clamp are
/// [`staking_protocol::epoch_at_block`] — the same code the node computes its
/// epochs with. What stays here is the ONE rule that is the contract's alone: a
/// zero activation block is the unarmed sentinel, not "armed at genesis". Without
/// it an unsigned `block_number < 0` is never true, so an unarmed chain counts
/// epochs from genesis and then drops them back to zero the moment a real
/// activation block is scheduled — a backwards jump that rewrites which
/// delegation checkpoint a stake belongs to. `ensure_dpos_not_active` keeps the
/// governance setters open on exactly that state, and the devnet production path
/// initializes into it deliberately.
///
/// The node does NOT read a zero activation this way, and does not have to: its
/// `scheduled_dpos_activation` folds a zero to "not a DPoS chain yet" and skips
/// the whole epoch section, so the input never reaches its epoch math.
pub fn epoch_at_block(block_number: u64, activation_block: u64, interval: u64) -> Option<u64> {
    if activation_block == 0 {
        // The unarmed sentinel is a CONTRACT-side rule, so it stays here rather
        // than in the shared formula. `ensure_dpos_not_active` keeps the
        // governance setters open on a zero activation; the node never reaches
        // this input at all, because `scheduled_dpos_activation` folds a zero to
        // "not a DPoS chain yet" before any epoch is computed. Interval zero is
        // still answered by the shared function, so the two sides cannot
        // disagree about it.
        return staking_protocol::epoch_at_block(block_number, activation_block, interval)
            .map(|_| 0);
    }
    staking_protocol::epoch_at_block(block_number, activation_block, interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_balance_rejects_precision_dust() {
        assert_eq!(
            compact_balance(U256::from(10_000_000_000u64)),
            Some(U112::ONE)
        );
        assert_eq!(compact_balance(U256::from(10_000_000_001u64)), None);
        let overflow = (U256::from(U112::MAX) + U256::ONE) * BALANCE_COMPACT_PRECISION;
        assert_eq!(compact_balance(overflow), None);
        assert_eq!(
            expand_balance(U112::MAX),
            overflow - BALANCE_COMPACT_PRECISION
        );
    }

    #[test]
    fn reward_narrowing_rejects_uint96_overflow() {
        assert_eq!(narrow_reward(U256::from(U96::MAX)), Some(U96::MAX));
        assert_eq!(narrow_reward(U256::from(U96::MAX) + U256::ONE), None);
    }

    #[test]
    fn epoch_is_rebased_and_clamped_before_activation() {
        assert_eq!(epoch_at_block(99, 100, 20), Some(0));
        assert_eq!(epoch_at_block(100, 100, 20), Some(0));
        assert_eq!(epoch_at_block(139, 100, 20), Some(1));
        assert_eq!(epoch_at_block(140, 100, 20), Some(2));
        assert_eq!(epoch_at_block(140, 100, 0), None);
    }

    #[test]
    fn unarmed_activation_pins_the_epoch_regardless_of_height() {
        assert_eq!(epoch_at_block(0, 0, 200), Some(0));
        assert_eq!(epoch_at_block(4_000, 0, 200), Some(0));
        assert_eq!(epoch_at_block(u64::MAX, 0, 200), Some(0));
    }
}
