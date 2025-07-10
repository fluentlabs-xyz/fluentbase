#[allow(deprecated)]
use {core::mem::size_of, solana_rbpf::memory_region::AccessType};

pub mod cpi;

/// Maximum signers
pub const MAX_SIGNERS: usize = 16;

/// Maximum CPI instruction data size. 10 KiB was chosen to ensure that CPI
/// instructions are not more limited than transaction instructions if the size
/// of transactions is doubled in the future.
pub const MAX_CPI_INSTRUCTION_DATA_LEN: u64 = 10 * 1024;

/// Maximum CPI instruction accounts. 255 was chosen to ensure that instruction
/// accounts are always within the maximum instruction account limit for SBF
/// program instructions.
pub const MAX_CPI_INSTRUCTION_ACCOUNTS: u8 = u8::MAX;

/// Maximum number of account info structs that can be used in a single CPI
/// invocation. A limit on account info structs is effectively the same as
/// limiting the number of unique accounts. 128 was chosen to match the max
/// number of locked accounts per transaction (MAX_TX_ACCOUNT_LOCKS).
pub const MAX_CPI_ACCOUNT_INFOS: usize = 128;
