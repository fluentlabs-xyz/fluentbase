//! The native loader native program.

use crate::account::{Account, AccountSharedData, InheritableAccountFields};
use solana_clock::INITIAL_RENT_EPOCH;
use solana_pubkey::{declare_id, Pubkey};

declare_id!("NativeLoader1111111111111111111111111111111");

pub const INHERITABLE_ACCOUNT_FIELDS: (u64, u64) = (0, INITIAL_RENT_EPOCH);

// /// Create an executable account with the given shared object name.
// #[deprecated(
//     since = "1.5.17",
//     note = "Please use `create_loadable_account_for_test` instead"
// )]
// pub fn create_loadable_account(name: &str, lamports: u64, owner: &Pubkey) -> AccountSharedData {
//     create_loadable_account_with_fields(name, owner, (lamports, INITIAL_RENT_EPOCH))
// }

pub fn create_loadable_account_with_fields(
    name: &str,
    owner: &Pubkey,
    (lamports, rent_epoch): InheritableAccountFields,
) -> AccountSharedData {
    AccountSharedData::from(Account {
        lamports,
        owner: *owner,
        data: name.as_bytes().to_vec(),
        executable: true,
        rent_epoch,
    })
}

pub fn create_loadable_account_with_fields2(name: &str, owner: &Pubkey) -> AccountSharedData {
    create_loadable_account_with_fields(name, owner, INHERITABLE_ACCOUNT_FIELDS)
}
