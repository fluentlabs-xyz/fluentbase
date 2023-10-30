use crate::{
    eth_types::*,
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
    BlockRlpReceiptsA = 10,
    BlockRlpReceiptsB = 11,
    BlockRlpStateLeavesA = 12, // MPT
    BlockRlpTxsA = 13,
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
    let empty_vec: Vec<u8> = Vec::new();
    // reading a block_a
    let block_txs_a_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockA as usize);
    if block_txs_a_rlp_endecoded.eq(&empty_vec) {
        // TODO
        panic!("EMPTY INPUT BLOCK_A");
    }
    let block_txs_a = rlp::decode::<block::Block>(&block_txs_a_rlp_endecoded).unwrap();

    // reading a block_b
    let block_txs_b_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockB as usize);
    if block_txs_b_rlp_endecoded.eq(&empty_vec) {
        // TODO
        panic!("EMPTY INPUT BLOCK_B");
    }
    let block_txs_b = rlp::decode::<block::Block>(&block_txs_b_rlp_endecoded).unwrap();

    // initial verification on blocks:
    let res = block::verify_input_blocks(&block_txs_a, &block_txs_b);
    if res.is_err() {
        panic!("{:?}", res.err().unwrap());
    }

    Ok(())
}

pub(crate) fn evm_verify_block_receipts(caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
    let block_txs_a_rlp_endecoded = caller.data().input.clone();

    let block_txs_a_rlp_endecoded_x = caller
        .data()
        .input(EvmInputSpec::BlockRlpReceiptsA as usize);
    let transaction_a =
        rlp::decode::<transaction::Transaction>(&block_txs_a_rlp_endecoded_x).unwrap();

    let block_txs_b_rlp_endecoded_x = caller
        .data()
        .input(EvmInputSpec::BlockRlpReceiptsB as usize);
    let transaction_b =
        rlp::decode::<transaction::Transaction>(&block_txs_b_rlp_endecoded_x).unwrap();

    let empty_vec: Vec<u8> = Vec::new();
    if block_txs_a_rlp_endecoded[0].eq(&empty_vec) {
        panic!("EMPTY INPUT");
    }

    zktrie_open(caller).unwrap();
    // zktrie_get_nonce(caller, key_offset, key_len, output_offset)

    let trie_root = zktrie_get_trie(&0).unwrap();

    // initial verification on transactions:
    // let res = verify_input_blocks(&block_txs_a, &block_txs_b);
    // if res.is_err() {
    //     panic!("{:?}", res.err().unwrap());
    // }

    Ok(())
}

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
