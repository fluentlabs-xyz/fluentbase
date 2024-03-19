use fluentbase_sdk::Bytes32;

/// Number of fields
pub(crate) const JZKT_ACCOUNT_FIELDS_COUNT: u32 = 6;

pub(crate) const JZKT_ACCOUNT_BALANCE_FIELD: u32 = 0;
pub(crate) const JZKT_ACCOUNT_NONCE_FIELD: u32 = 1;
pub(crate) const JZKT_ACCOUNT_SOURCE_BYTECODE_SIZE_FIELD: u32 = 2;
pub(crate) const JZKT_ACCOUNT_SOURCE_BYTECODE_HASH_FIELD: u32 = 3;
pub(crate) const JZKT_ACCOUNT_RWASM_BYTECODE_SIZE_FIELD: u32 = 4;
pub(crate) const JZKT_ACCOUNT_RWASM_BYTECODE_HASH_FIELD: u32 = 5;

/// Compression flags for upper fields.
///
/// We compress following fields:
/// - balance (0) because of balance overflow
/// - source code hash (3) because its keccak256
///
/// Mask is: 0b00001001
pub const JZKT_COMPRESSION_FLAGS: u32 =
    (1 << JZKT_ACCOUNT_BALANCE_FIELD) + (1 << JZKT_ACCOUNT_SOURCE_BYTECODE_HASH_FIELD);

/// EIP-170: Contract code size limit
///
/// By default this limit is 0x6000 (~24kb)
pub(crate) const MAX_BYTECODE_SIZE: u32 = 0x6000;

pub type AccountCheckpoint = (u32, u32);
pub type AccountFields = [Bytes32; JZKT_ACCOUNT_FIELDS_COUNT as usize];
pub type Topics<const TOPICS_COUNT: usize> = [Bytes32; TOPICS_COUNT];
