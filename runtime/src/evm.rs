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

pub(crate) fn evm_verify_rlp_blocks(caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
    let block_txs_a_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockA as usize);
    let block_txs_a = rlp::decode::<eth_types::block::Block>(&block_txs_a_rlp_endecoded).unwrap();

    println!("block_a_nubmer: {:?}", block_txs_a.header.number());

    let block_txs_b_rlp_endecoded = caller.data().input(EvmInputSpec::RlpBlockB as usize);
    let block_txs_b = rlp::decode::<eth_types::block::Block>(&block_txs_b_rlp_endecoded).unwrap();

    println!("block_a_nubmer: {:?}", block_txs_b.header.number());

    // // initial verification on blocks:
    // let res = eth_types::block::verify_input_blocks(&block_txs_a, &block_txs_b);
    // if res.is_err() {
    //     let err = res.err().unwrap();
    //     //return Err(eth_types::block::VerifyBlockError::ParentHashWrong);
    //     // return Err(res.err());
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

#[cfg(test)]
mod tests {
    use crate::{
        eth_types::{
            block::Block,
            header::{generate_random_header, generate_random_header_based_on_prev_block},
        },
        evm_verify_rlp_blocks,
        tests::wat2rwasm,
        Runtime,
    };
    use keccak_hash::H256;

    #[test]
    fn test_verify_block_rlp() {
        // 1. generate block rlp and put read it with evm_rlp_block_a
        let blk_a_header = generate_random_header(&123120);
        let blk_a = Block {
            header: blk_a_header,
            transactions: vec![],
            uncles: vec![],
        };
        let blk_a_encoded = rlp::encode(&blk_a);

        // 2. current block
        let blk_b_header = generate_random_header_based_on_prev_block(&123121, H256::random());
        let blk_b = Block {
            header: blk_b_header,
            transactions: vec![],
            uncles: vec![],
        };
        let blk_b_encoded = rlp::encode(&blk_b);

        let rwasm_binary = wat2rwasm(&format!(
            r#"
    (module
      (type (;0;) (func (param i32 i32 i32)))
      (type (;1;) (func))
      (type (;2;) (func (param i32 i32)))
      (import "env" "_verify_rlp_block_a" (func $_verify_rlp_block_a (type 0)))
      (import "env" "_evm_return" (func $_evm_return (type 2)))
      (func $main (type 1)
        i32.const 0
        i32.const 12
        i32.const 50
        call $_evm_keccak256
        i32.const 50
        i32.const 32
        call $_evm_return
        )
      (memory (;0;) 100)
      (data (;0;) (i32.const 0) "{}")
      (data (;0;) (i32.const 0) "{}")
      (export "main" (func $main)))
        "#,
            543,
            123,
            /* blk_a_encoded.to_vec(),
             * blk_b_encoded.to_vec(), */
        ));

        let result = Runtime::run(rwasm_binary.as_slice(), &[]).unwrap();
        println!("{:?}", result);
        match hex::decode("0xa04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529") {
            Ok(answer) => {
                assert_eq!(&answer, result.data().output().as_slice());
            }
            Err(e) => {
                // If there's an error, you might want to handle it in some way.
                // For this example, I'll just print the error.
                println!("Error: {:?}", e);
            }
        }

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
