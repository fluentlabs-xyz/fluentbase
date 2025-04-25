use crate::solana_program::sysvar::Sysvar;
pub use crate::{account_info::AccountInfo, solana_program::program_error::ProgramError};
pub use solana_slot_history::{
    sysvar::{check_id, id, ID},
    SlotHistory,
};

impl Sysvar for SlotHistory {
    // override
    fn size_of() -> usize {
        // hard-coded so that we don't have to construct an empty
        131_097 // golden, update if MAX_ENTRIES changes
    }
    fn from_account_info(_account_info: &AccountInfo) -> Result<Self, ProgramError> {
        // This sysvar is too large to bincode_deserialize in-program
        Err(ProgramError::UnsupportedSysvar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_bincode::serialized_size;
    #[test]
    fn test_size_of() {
        assert_eq!(
            SlotHistory::size_of(),
            serialized_size(&SlotHistory::default()).unwrap() as usize
        );
    }
}
