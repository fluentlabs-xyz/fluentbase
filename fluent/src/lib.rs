mod eth_typ;
mod eth_types;
pub mod evm;
mod hash;
#[cfg(test)]
mod tests;

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

// #[no_mangle]
pub extern "C" fn main() {
    fn verify_block_transition() {
        // let mut mem = vec![0u8; 1024 * 1024];
        // let rlp_block_a_len = sys_input(
        //     EvmInputSpec::RlpBlockA as u32,
        //     mem.as_mut_ptr() as u32,
        //     0,
        //     0,
        // );
        // let rlp_block_b_len = sys_input(
        //     EvmInputSpec::RlpBlockB as u32,
        //     mem.as_mut_ptr() as u32,
        //     rlp_block_a_len as u32,
        //     0,
        // );
        // let rlp_block_a = &mem[0..rlp_block_a_len as usize];
        // let rlp_block_a =
        //     &mem[rlp_block_a_len as usize..(rlp_block_a_len + rlp_block_b_len) as usize];

        // let block_a = decode_rlp(rlp_block_a);
        // let block_b = decode_rlp(rlp_block_b);

        // let block_state_leaves_b = rlp_decode(sys_input(BlockRlpStateLeavesB));
        // let block_txs_a = rlp_decode(sys_input(BlockTxsA));
        // let block_receipts_a = rlp_decode(sys_input(BlockReceiptsA));

        // // check root
        // zktrie_open();
        // // TODO update zktrie with current state
        // for (i, raw_tx) in block_txs_b {
        //     let tx = rlp_decode(raw_tx);
        //     let receipt = EVM::transact(tx);
        //     assert(hash(receipt) == receipts[i]);
        // }
        // assert(block_b.root == zktrie_root());

        // // check tx hash
        // mpt_open(); // reset
        // for (i, tx) in &block_txs_a {
        //     let key = rlp_encode(i);
        //     mpt_update(key.ptr, key.len, tx.ptr, tx.len);
        // }
        // assert(block_a.tx_hash == mpt_root());

        // // check tx hash
        // mpt_open(); // reset
        // for (i, tx) in &block_receipts_a {
        //     let key = rlp_encode(i);
        //     mpt_update(key.ptr, key.len, tx.ptr, tx.len);
        // }
        // assert(block_a.receipt_hash == mpt_root());
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn test_main() {}
}
