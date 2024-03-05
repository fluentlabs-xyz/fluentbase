use fluentbase_sdk::Bytes32;

pub(crate) const JZKT_ACCOUNT_FIELDS_COUNT: u32 = 7;

pub(crate) const JZKT_ACCOUNT_ROOT_FIELD: u32 = 0;
pub(crate) const JZKT_ACCOUNT_NONCE_FIELD: u32 = 1;
pub(crate) const JZKT_ACCOUNT_BALANCE_FIELD: u32 = 2;
pub(crate) const JZKT_ACCOUNT_CODE_SIZE_FIELD: u32 = 3;
pub(crate) const JZKT_ACCOUNT_CODE_HASH_FIELD: u32 = 4;
pub(crate) const JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD: u32 = 5;
pub(crate) const JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD: u32 = 6;

/// Compression flags for upper fields, we compress
/// only code hash and balance fields (0b1100)
pub const JZKT_COMPRESSION_FLAGS: u32 = 0b1100;

/// EIP-170: Contract code size limit
///
/// By default this limit is 0x6000 (~24kb)
pub(crate) const MAX_CODE_SIZE: u32 = 0x6000;
pub type AccountCheckpoint = (u32, u32);
pub type AccountFields = [Bytes32; JZKT_ACCOUNT_FIELDS_COUNT as usize];
pub type Topics<const TOPICS_COUNT: usize> = [Bytes32; TOPICS_COUNT];
