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

pub(crate) fn evm_block_number(
    mut caller: Caller<'_, RuntimeContext>,
    ptr: u32,
) -> Result<(), Trap> {
    // let buff = caller.data().input(EvmInputSpec::BlockNumberA as usize);
    // let block = exported_memory_slice(&mut caller, ptr as usize, 8);
    // block.copy_from_slice(buff.as_slice());
    Ok(())
}

pub(crate) fn evm_block_hash(mut caller: Caller<'_, RuntimeContext>, ptr: u32) {
    // let buff = caller.data().input(EvmInputSpec::BlockHashA as usize);
    // let block = exported_memory_slice(&mut caller, ptr as usize, 8);
    // block.copy_from_slice(buff.as_slice());
}
