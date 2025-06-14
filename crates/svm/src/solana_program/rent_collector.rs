//! calculate and collect rent from Accounts

use solana_clock::Epoch;

/// When rent is collected from an exempt account, rent_epoch is set to this
/// value. The idea is to have a fixed, consistent value for rent_epoch for all accounts that do not collect rent.
/// This enables us to get rid of the field completely.
pub const RENT_EXEMPT_RENT_EPOCH: Epoch = Epoch::MAX;
