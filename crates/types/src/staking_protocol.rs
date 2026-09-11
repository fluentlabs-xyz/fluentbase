//! Protocol limits and arithmetic that the staking contract and the node must
//! agree on.
//!
//! Everything here used to exist twice — once in `contracts/staking/src` and
//! once under `crates/dpos` — with a comment asking the next editor to keep the
//! two literals in step. A comment is not a mechanism: both copies were only
//! ever equal because nobody had changed one of them yet. These are the single
//! declarations; both sides import them, so a change is a compile error on the
//! side that did not follow rather than a silent disagreement on a live chain.
//!
//! The ABI surface (selectors, return shapes, event layouts) has its own home:
//! `fluentbase-staking-abi`. This module is deliberately free of
//! `alloy-sol-types` so it can stay a leaf of `fluentbase-types`.

use alloy_primitives::U256;

/// Smallest committee `commitEpochCommittee` will accept.
///
/// Four is the smallest `n` for which [`fault_tolerance`] returns `f >= 1`, so
/// it is the smallest committee that tolerates a single fault. The contract
/// enforces it on the way in (`setActiveValidatorsLength` refuses a cap under
/// it) and on the way out (a short selection reverts `CommitteeTooSmall`); the
/// node asserts the same floor when it decodes a committee, because a non-empty
/// committee shorter than this cannot be a legal on-chain state.
pub const MIN_COMMITTEE_LENGTH: usize = 4;

/// Hard ceiling on the committee / configured active-set size.
///
/// One name for what used to be two: the contract called it
/// `MAX_ACTIVE_VALIDATORS_LENGTH` (the cap `setActiveValidatorsLength` enforces
/// and the selection truncates to) and the node called it `MAX_COMMITTEE_SIZE`
/// (the certificate-bitmap codec bound and the `leader_index: u8` wire limit).
/// They are the same number for the same reason, and a committee larger than
/// the codec bound is undecodable on every node at once.
///
/// Must stay `<= 255` while the production record encodes a committee position
/// in one byte.
pub const MAX_COMMITTEE_SIZE: u64 = 51;

/// Width of the compact stake unit, in bits.
///
/// The contract stores `totalDelegated` as a `uint112` count of
/// [`BALANCE_COMPACT_PRECISION`] units; the node rejects anything at or above
/// [`MAX_COMPACT_STAKE`] rather than carrying it into the leader elector, which
/// is the bound `WeightedVrf::build` cites when it argues its prefix-sum cannot
/// overflow (51 members × `< 2^112` ≈ `2^119` ≪ `u128::MAX`).
pub const COMPACT_STAKE_BITS: usize = 112;

/// One past the largest legal compact stake weight, i.e. `2^112`.
pub const MAX_COMPACT_STAKE: u128 = 1u128 << COMPACT_STAKE_BITS;

/// Wei per compact stake unit: `10^10`.
///
/// An amount that is not a whole multiple of it cannot be represented and is
/// rejected on the contract side; the node divides by it to recover the
/// compacted weight the elector ranks on. A drift here mis-weights leaders
/// silently — every node equally wrong, so there is no fork to notice.
pub const BALANCE_COMPACT_PRECISION: u128 = 10_000_000_000;

/// [`BALANCE_COMPACT_PRECISION`] as a `U256`, for the contract's arithmetic.
///
/// Derived, not a second literal: `10^10` fits a single limb.
pub const BALANCE_COMPACT_PRECISION_U256: U256 =
    U256::from_limbs([BALANCE_COMPACT_PRECISION as u64, 0, 0, 0]);

/// How far ahead of the current epoch a committee may be committed, and — the
/// same number, because they are the same offset — how far back of the target
/// epoch its membership is selected from.
///
/// The node's ahead-commit loop drains up to `current_epoch + this`; the
/// contract refuses a commit beyond it. A node horizon larger than the
/// contract's is a revert on a fail-loud system call, i.e. a halted chain.
pub const MAX_COMMITTEE_LOOKAHEAD_EPOCHS: u64 = 2;

/// Epochs the contract's frozen-weight ring holds before a frame is reused.
///
/// `commitEpochCommittee` stamps each committed epoch's leader weights into
/// slot `epoch mod` this number; past the wrap the contract answers an EMPTY
/// `stakes` leg, which the reader decodes as
/// `ValidatorSetSnapshot::weights = None`. Membership and keys are retained
/// forever — only the weights expire — so the two legs have genuinely
/// different lifetimes.
///
/// Shared rather than contract-local because the NODE pins an invariant
/// against it: the committee module reads an epoch only inside
/// `[epoch(anchor) − SCHEME_RETENTION_EPOCHS, epoch(anchor) +
/// MAX_COMMITTEE_LOOKAHEAD_EPOCHS]`, and it is
/// `WEIGHT_RING_EPOCHS − MAX_COMMITTEE_LOOKAHEAD_EPOCHS >
/// SCHEME_RETENTION_EPOCHS` that makes the weights of every epoch in that
/// window still present, hence `CommitteeRecord::weights` non-optional. That
/// inequality is a `const _: () = assert!(…)` on the node side, so shrinking
/// the ring here is a compile error there rather than a run-time `None` the
/// leader elector would have to invent a uniform lottery for.
pub const WEIGHT_RING_EPOCHS: u64 = 16;

/// Compressed BLS12-381 G2 public key (MinSig), in bytes.
pub const BLS_PUBKEY_LENGTH: usize = 96;
/// Compressed BLS12-381 G1 signature (MinSig), in bytes.
pub const BLS_SIGNATURE_LENGTH: usize = 48;
/// EIP-2537 uncompressed G2 public key: 4 × 64 bytes.
pub const BLS_PUBKEY_UNCOMPRESSED_LENGTH: usize = 256;
/// EIP-2537 uncompressed G1 signature (and proof of possession): 2 × 64 bytes.
pub const BLS_SIGNATURE_UNCOMPRESSED_LENGTH: usize = 128;

/// Bytes of proposal payload one equivocation-evidence record carries — the
/// consensus block digest.
///
/// The contract's evidence decoder reads exactly this many bytes per proposal;
/// the node's `Digest` is exactly this wide. Evidence built against a different
/// length does not decode, and the slash is lost without a sound.
pub const PROPOSAL_PAYLOAD_LENGTH: usize = 32;

/// Simplex fault tolerance `f = ⌊(n−1)/3⌋`.
///
/// The same budget commonware's `N3f1::max_faults` applies to every Simplex
/// quorum. `n == 0` answers `0` here where commonware panics; no caller reaches
/// it, and returning a budget for an empty set is the wrong shape of answer.
/// [`MIN_COMMITTEE_LENGTH`] is defined as the smallest `n` this returns `>= 1`
/// for, so the two move together.
pub const fn fault_tolerance(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        (n - 1) / 3
    }
}

/// Map a block to its activation-relative epoch.
///
/// `None` for a zero interval — the caller decides whether that is a
/// misconfiguration or a not-yet-armed chain. Pre-activation blocks clamp to
/// epoch `0` rather than underflowing.
///
/// This is the formula only. The "activation block `0` is the *unarmed*
/// sentinel, not `armed at genesis`" rule is the contract's own policy and
/// lives with the contract: the node never reaches it, because
/// `scheduled_dpos_activation` folds a zero activation to "not a DPoS chain
/// yet" before any epoch is computed.
pub const fn epoch_at_block(
    block_number: u64,
    activation_block: u64,
    interval: u64,
) -> Option<u64> {
    if interval == 0 {
        return None;
    }
    Some(block_number.saturating_sub(activation_block) / interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committee_floor_is_the_smallest_single_fault_committee() {
        assert_eq!(fault_tolerance(MIN_COMMITTEE_LENGTH), 1);
        assert_eq!(fault_tolerance(MIN_COMMITTEE_LENGTH - 1), 0);
    }

    #[test]
    fn the_committee_cap_fits_the_one_byte_leader_index() {
        assert!(MAX_COMMITTEE_SIZE <= u8::MAX as u64);
    }

    #[test]
    fn the_u256_precision_is_the_same_number_as_the_u128_one() {
        assert_eq!(
            BALANCE_COMPACT_PRECISION_U256,
            U256::from(BALANCE_COMPACT_PRECISION)
        );
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
