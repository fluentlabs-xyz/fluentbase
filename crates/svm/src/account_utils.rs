//! Useful extras for `Account` state.

use crate::{
    account::{Account, AccountSharedData, ReadableAccount},
    solana_program::sysvar::Sysvar,
};
use bincode::error::EncodeError;
use core::cell::Ref;
use solana_bincode::deserialize;
use solana_instruction::error::InstructionError;

/// Convenience trait to covert bincode errors to instruction errors.
pub trait StateMut<T> {
    fn state(&self) -> Result<T, InstructionError>;
    fn set_state(&mut self, state: &T) -> Result<(), InstructionError>;
}
pub trait State<T> {
    fn state(&self) -> Result<T, InstructionError>;
    fn set_state(&self, state: &T) -> Result<(), InstructionError>;
}

macro_rules! impl_state_for {
    ($typ:ty) => {
        impl<T> StateMut<T> for $typ
        where
            T: serde::Serialize + serde::de::DeserializeOwned,
        {
            fn state(&self) -> Result<T, InstructionError> {
                self.deserialize_data()
                    .map_err(|_| InstructionError::InvalidAccountData)
            }
            fn set_state(&mut self, state: &T) -> Result<(), InstructionError> {
                self.serialize_data(state)
                    .map_err(|ref err| match err {
                        EncodeError::Other("account data size limit") => {
                            InstructionError::AccountDataTooSmall
                        }
                        _ => InstructionError::GenericError,
                    })
                    .map(|_| ())
            }
        }
    };
}

impl_state_for!(Account);
impl_state_for!(AccountSharedData);

impl<T> StateMut<T> for Ref<'_, AccountSharedData>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    fn state(&self) -> Result<T, InstructionError> {
        self.deserialize_data()
            .map_err(|_| InstructionError::InvalidAccountData)
    }
    fn set_state(&mut self, _state: &T) -> Result<(), InstructionError> {
        Err(InstructionError::Immutable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_account_state() {
        let state = 42u64;

        assert!(AccountSharedData::default().set_state(&state).is_err());
        let res: Result<u64, InstructionError> = AccountSharedData::default().state();
        assert!(res.is_err());

        let mut account = AccountSharedData::new(0, size_of::<u64>(), &Pubkey::default());

        assert!(account.set_state(&state).is_ok());
        let stored_state: u64 = account.state().unwrap();
        assert_eq!(stored_state, state);
    }
}

/// Create a `Sysvar` from an `Account`'s data.
pub fn from_account<S: Sysvar, T: ReadableAccount>(account: &T) -> Option<S> {
    deserialize(account.data()).ok()
}
