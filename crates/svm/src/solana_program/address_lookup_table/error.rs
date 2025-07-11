#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AddressLookupError {
    /// Attempted to lookup addresses from a table that does not exist
    LookupTableAccountNotFound,
    /// Attempted to lookup addresses from an account owned by the wrong program
    InvalidAccountOwner,
    /// Attempted to lookup addresses from an invalid account
    InvalidAccountData,
    /// Address lookup contains an invalid index
    InvalidLookupIndex,
}
