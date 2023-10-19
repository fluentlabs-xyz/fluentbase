use crate::{
    eth_types,
    instruction::{exported_memory_slice, exported_memory_vec},
    sys_input,
    zktrie_get_trie,
    zktrie_open,
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

pub(crate) fn evm_rlp_block_a(
    mut caller: Caller<'_, RuntimeContext>,
    ptr: u32,
) -> Result<(), Trap> {
    let buff = caller.data().input(EvmInputSpec::RlpBlockA as usize);
    // let block_rlp = exported_memory_slice(&mut caller, ptr as usize, 8);
    // block_rlp.copy_from_slice(buff.as_slice());
    Ok(())
}

pub(crate) fn verify_rlp_block_a(
    mut caller: Caller<'_, RuntimeContext>,
    ptr: u32,
) -> Result<(), Trap> {
    // evm_rlp_block_a(caller, ptr).unwrap();
    let block_txs_a_rlp_decoded = caller.data().input(EvmInputSpec::RlpBlockA as usize);
    let block_txs_a = rlp::decode::<eth_types::block::Block>(&block_txs_a_rlp_decoded).unwrap();

    let block_txs_b_rlp_decoded = caller.data().input(EvmInputSpec::RlpBlockB as usize);
    let block_txs_b = rlp::decode::<eth_types::block::Block>(&block_txs_b_rlp_decoded).unwrap();

    // initial verification on blocks:
    let res = eth_types::block::verify_input_blocks(&block_txs_a, &block_txs_b);
    if res.is_err() {
        let err = res.err().unwrap();
        //return Err(eth_types::block::VerifyBlockError::ParentHashWrong);
        // return Err(res.err());
    }

    let block_receipts_a_decoded = caller
        .data()
        .input(EvmInputSpec::BlockRlpReceiptsA as usize);

    // let block_receipts_a = rlp_decode(sys_input(BlockReceiptsA));

    // // check root
    // zktrie_open(caller); // reset
    // for (i, raw_tx) in block_txs_a.transactions.iter() {
    //     // 1. decode tx into eth_types::Transaction
    //     //   let tx = rlp_decode(raw_tx);
    //     // 2. execute transaction
    //     // 3. verify a receipt
    //     // assert(hash(receipt) == receipts[i]);
    // }
    // let trie_root = zktrie_get_trie();

    // assert(block_a.root == zktrie_root());

    // let buff = caller.data().input(EvmInputSpec::RlpBlockA as usize);
    // let block_rlp = exported_memory_slice(&mut caller, ptr as usize, 8);
    // // block_rlp.copy_from_slice(buff.as_slice());
    // // Ok(block_rlp.as_b as i32)
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_verify_block_rlp() {

        // 1. generate block rlp and put read it with evm_rlp_block_a

        // verify_block_transition() {

        //     let rlp_block_a = sys_input(RlpBlockA);
        //     let rlp_block_b = sys_input(RlpBlockB);

        //     let block_a = decode_rlp(rlp_block_a);
        //     let block_b = decode_rlp(rlp_block_b);

        //     let block_state_leaves_b = rlp_decode(sys_input(BlockRlpStateLeavesB));
        //     let block_txs_a = rlp_decode(sys_input(BlockTxsA));
        //     let block_receipts_a = rlp_decode(sys_input(BlockReceiptsA));

        //     // check root
        //     zktrie_open(..initial trie...); // reset
        //     for (i, raw_tx) in block_txs_a {
        //       let tx = rlp_decode(raw_tx);
        //       let receipt = exec_tx(tx);
        //       assert(hash(receipt) == receipts[i]);
        //     }
        //     assert(block_a.root == zktrie_root());

        //     // check tx hash
        //     mpt_open(); // reset
        //     for (i, tx) in &block_txs_a {
        //       let key = rlp_encode(i);
        //       mpt_update(key.ptr, key.len, tx.ptr, tx.len);
        //     }
        //     assert(block_a.tx_hash == mpt_root());

        //     // check tx hash
        //     mpt_open(); // reset
        //     for (i, tx) in &block_receipts_a {
        //       let key = rlp_encode(i);
        //       mpt_update(key.ptr, key.len, tx.ptr, tx.len);
        //     }
        //     assert(block_a.receipt_hash == mpt_root());
        //   }
    }
}
