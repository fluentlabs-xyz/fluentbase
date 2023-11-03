use crate::{
    instruction::{exported_memory_slice, exported_memory_vec},
    RuntimeContext,
};
use fluentbase_rwasm::{common::Trap, Caller};
use keccak_hash::keccak;

enum EvmInputSpec {
    RlpBlockA = 0,
    RlpBlockB = 1,
    // add more public inputs
    // BlockParentHashA = 10,
    // BlockUncleHashA = 11,
    // BlockCoinbaseA = 12,
    BlockRlpStateLeavesA = 13, // MPT
    BlockRlpTxsA = 14,
    BlockRlpReceiptsA = 15,
    // BlockDifficultyA = 16,
    // BlockNumberA = 10,
    // BlockHashA = 11,
    // add more private inputs
}
