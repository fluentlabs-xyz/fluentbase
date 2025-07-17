use crate::compute_budget::compute_budget::HEAP_LENGTH;

pub const MIN_HEAP_FRAME_BYTES: u32 = HEAP_LENGTH as u32;
pub const MAX_HEAP_FRAME_BYTES: u32 = HEAP_LENGTH as u32 * 2i32.pow(8) as u32;
pub const DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT: u32 = 200_000;
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

/// The total accounts data a transaction can load is limited to 64MiB to not break
/// anyone in Mainnet-beta today. It can be set by set_loaded_accounts_data_size_limit instruction
pub const MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES: u32 = 64 * 1024 * 1024;
