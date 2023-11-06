use crate::eth_types::{block, transaction};

enum EvmInputSpec {
    RlpBlockA = 0,
    RlpBlockB = 1,
    // add more public inputs
    // BlockParentHashA = 10,
    // BlockUncleHashA = 11,
    // BlockCoinbaseA = 12,
    BlockRlpReceiptsA = 10,
    BlockRlpReceiptsB = 11,
    BlockRlpStateLeavesA = 12, // MPT
    BlockRlpTxsA = 13,
    // BlockHashA = 11,
    // add more private inputs
}

// pub fn evm_rlp_block_a(mut caller: Caller<'_, RuntimeContext>, ptr: u32) -> Result<(), Trap> {
//     let buff = caller.data().input(EvmInputSpec::RlpBlockA as usize);
//     // let block_rlp = exported_memory_slice(&mut caller, ptr as usize, 8);
//     // block_rlp.copy_from_slice(buff.as_slice());
//     Ok(())
// }

// pub fn evm_verify_block_rlps(caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
//     let empty_vec: Vec<u8> = Vec::new();
//     // reading a block_a
//     let block_txs_a_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockA as usize);
//     if block_txs_a_rlp_endecoded.eq(&empty_vec) {
//         // TODO
//         panic!("EMPTY INPUT BLOCK_A");
//     }
//     let block_txs_a = rlp::decode::<block::Block>(&block_txs_a_rlp_endecoded).unwrap();
//
//     // reading a block_b
//     let block_txs_b_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockB as usize);
//     if block_txs_b_rlp_endecoded.eq(&empty_vec) {
//         // TODO
//         panic!("EMPTY INPUT BLOCK_B");
//     }
//     let block_txs_b = rlp::decode::<block::Block>(&block_txs_b_rlp_endecoded).unwrap();
//
//     // initial verification on blocks:
//     let res = block::verify_input_blocks(&block_txs_a, &block_txs_b);
//     if res.is_err() {
//         panic!("{:?}", res.err().unwrap());
//     }
//
//     Ok(())
// }

// pub fn evm_verify_block_receipts(caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
//     let block_txs_a_rlp_endecoded = caller.data().input(0).clone();
//
//     let block_txs_a_rlp_endecoded_x = caller
//         .data()
//         .input(EvmInputSpec::BlockRlpReceiptsA as usize);
//     let transaction_a =
//         rlp::decode::<transaction::Transaction>(&block_txs_a_rlp_endecoded_x).unwrap();
//
//     let block_txs_b_rlp_endecoded_x = caller
//         .data()
//         .input(EvmInputSpec::BlockRlpReceiptsB as usize);
//     let transaction_b =
//         rlp::decode::<transaction::Transaction>(&block_txs_b_rlp_endecoded_x).unwrap();
//
//     if block_txs_a_rlp_endecoded.is_empty() {
//         panic!("EMPTY INPUT");
//     }
//
//     // new state:
//
//     // B_a -> B_b
//
//     // mpt_open()?;
//
//     // mpt_update(caller, key_offset, key_len, value_offset, value_len).unwrap();
//     // zktrie_get_nonce(caller, key_offset, key_len, output_offset)
//     // zktrie_update_balance(caller, key_offset, key_len, value_offset, value_len)
//
//     // let trie_root = zktrie_get_root(&TRIE_ID_DEFAULT)?;
//
//     // initial verification on transactions:
//     // let res = verify_input_blocks(&block_txs_a, &block_txs_b);
//     // if res.is_err() {
//     //     panic!("{:?}", res.err().unwrap());
//     // }
//
//     Ok(())
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
