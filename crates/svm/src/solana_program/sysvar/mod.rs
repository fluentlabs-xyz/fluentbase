use crate::solana_program::program_error::ProgramError;
#[deprecated(since = "2.1.0", note = "Use `solana-sysvar-id` crate instead")]
pub use solana_sysvar_id::{
    check_id,
    declare_deprecated_sysvar_id,
    declare_sysvar_id,
    id,
    SysvarId,
    ID,
};
#[allow(deprecated)]
pub use sysvar_ids::ALL_IDS;

pub mod clock;
pub mod epoch_rewards;
pub mod epoch_schedule;
pub mod instructions;
pub mod rent;
use solana_account_info::AccountInfo;
use solana_bincode::{deserialize, serialize_into, serialized_size};
use solana_pubkey::Pubkey;

#[deprecated(
    since = "2.0.0",
    note = "please use `solana_sdk::reserved_account_keys::ReservedAccountKeys` instead"
)]
mod sysvar_ids {
    use super::*;
    use alloc::{vec, vec::Vec};
    use lazy_static::lazy_static;
    use solana_pubkey::Pubkey;

    lazy_static! {
        // This will be deprecated and so this list shouldn't be modified
        pub static ref ALL_IDS: Vec<Pubkey> = vec![
            clock::id(),
            epoch_schedule::id(),
            #[allow(deprecated)]
            rent::id(),
            instructions::id(),
        ];
    }
}

/// Returns `true` of the given `Pubkey` is a sysvar account.
#[deprecated(
    since = "2.0.0",
    note = "please check the account's owner or use solana_sdk::reserved_account_keys::ReservedAccountKeys instead"
)]
#[allow(deprecated)]
pub fn is_sysvar_id(id: &Pubkey) -> bool {
    ALL_IDS.iter().any(|key| key == id)
}

/// A type that holds sysvar data.
pub trait Sysvar:
    SysvarId + Default + Sized + serde::Serialize + serde::de::DeserializeOwned
{
    /// The size in bytes of the sysvar as serialized account data.
    fn size_of() -> usize {
        serialized_size(&Self::default()).unwrap()
    }

    /// Deserializes the sysvar from its `AccountInfo`.
    ///
    /// # Errors
    ///
    /// If `account_info` does not have the same ID as the sysvar this function
    /// returns [`ProgramError::InvalidArgument`].
    fn from_account_info(account_info: &AccountInfo) -> Result<Self, ProgramError> {
        if !Self::check_id(account_info.unsigned_key()) {
            return Err(ProgramError::InvalidArgument);
        }
        deserialize(&account_info.data.borrow()).map_err(|_| ProgramError::InvalidArgument)
    }

    /// Serializes the sysvar to `AccountInfo`.
    ///
    /// # Errors
    ///
    /// Returns `None` if serialization failed.
    fn to_account_info(&self, account_info: &mut AccountInfo) -> Option<usize> {
        serialize_into(self, &mut account_info.data.borrow_mut()[..]).ok()
    }

    /// Load the sysvar directly from the runtime.
    ///
    /// This is the preferred way to load a sysvar. Calling this method does not
    /// incur any deserialization overhead, and does not require the sysvar
    /// account to be passed to the program.
    ///
    /// Not all sysvars support this method. If not, it returns
    /// [`ProgramError::UnsupportedSysvar`].
    fn get() -> Result<Self, ProgramError> {
        Err(ProgramError::UnsupportedSysvar)
    }
}

/// Implements the [`Sysvar::get`] method for both SBF and host targets.
#[macro_export]
macro_rules! impl_sysvar_get {
    ($syscall_name:ident) => {
        fn get() -> Result<Self, ProgramError> {
            let mut var = Self::default();
            let var_addr = &mut var as *mut _ as *mut u8;

            let result = $crate::solana_program::program_stubs::$syscall_name(var_addr);

            match result {
                solana_program_entrypoint::SUCCESS => Ok(var),
                e => Err(e.into()),
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana_program::program_error::ProgramError;
    use alloc::rc::Rc;
    use core::cell::RefCell;
    use serde::{Deserialize, Serialize};
    use solana_clock::Epoch;
    use solana_pubkey::{declare_id, Pubkey};

    #[repr(C)]
    #[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
    struct TestSysvar {
        something: Pubkey,
    }
    declare_id!("TestSysvar111111111111111111111111111111111");
    impl SysvarId for TestSysvar {
        fn id() -> Pubkey {
            id()
        }

        fn check_id(pubkey: &Pubkey) -> bool {
            check_id(pubkey)
        }
    }
    impl Sysvar for TestSysvar {}

    #[test]
    fn test_sysvar_account_info_to_from() {
        let test_sysvar = TestSysvar::default();
        let key = id();
        let wrong_key = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mut lamports = 42;
        let mut data = vec![0_u8; TestSysvar::size_of()];
        let mut account_info = AccountInfo::new(
            &key,
            false,
            true,
            &mut lamports,
            &mut data,
            &owner,
            false,
            Epoch::default(),
        );

        test_sysvar.to_account_info(&mut account_info).unwrap();
        let new_test_sysvar = TestSysvar::from_account_info(&account_info).unwrap();
        assert_eq!(test_sysvar, new_test_sysvar);

        account_info.key = &wrong_key;
        assert_eq!(
            TestSysvar::from_account_info(&account_info),
            Err(ProgramError::InvalidArgument)
        );

        let mut small_data = vec![];
        account_info.data = Rc::new(RefCell::new(&mut small_data));
        assert_eq!(test_sysvar.to_account_info(&mut account_info), None);
    }
}
