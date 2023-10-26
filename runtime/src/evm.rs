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
    BlockNumberA = 10,
    // BlockHashA = 11,
    // add more private inputs
}

pub(crate) fn evm_block_number(
    mut caller: Caller<'_, RuntimeContext>,
    data_offset: i32,
    data_len: i32,
    output_offset: i32,
) -> Result<(), Trap> {
    // let buff = caller.data().input(EvmInputSpec::BlockNumberA as usize);
    let block_num = exported_memory_vec(&mut caller, data_offset as usize, data_len as usize);
    //block_data.copy_from_slice(buff.as_slice());
    caller.write_memory(output_offset as usize, block_num.as_slice());

    Ok(())
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

pub(crate) fn evm_verify_block_rlps(caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
    let block_txs_a_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockA as usize);
    let block_txs_a = rlp::decode::<eth_types::block::Block>(&block_txs_a_rlp_endecoded).unwrap();

    // let empty_vec: Vec<u8> = Vec::new();
    // if block_txs_a_rlp_endecoded.to_vec().gt(&empty_vec.to_vec()) {
    //     panic!("EMPTY");
    // }

    // let block_txs_b_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockB as usize);
    // let block_txs_b =
    // rlp::decode::<eth_types::block::Block>(&block_txs_b_rlp_endecoded).unwrap();

    // assert_eq!(block_txs_a_rlp_endecoded, block_txs_b_rlp_endecoded);

    // // initial verification on blocks:
    // let res = eth_types::block::verify_input_blocks(&block_txs_a, &block_txs_b);
    // if res.is_err() {
    //     let err = res.err().unwrap();
    //     //return Err(err.into());
    // }

    // let block_receipts_a_decoded = caller
    //     .data()
    //     .input(EvmInputSpec::BlockRlpReceiptsA as usize);

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
