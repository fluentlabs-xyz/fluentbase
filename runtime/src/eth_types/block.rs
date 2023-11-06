// use super::header::Header;
use crate::{
    eth_typ::transactions::*,
    eth_types::{header, transaction},
};
use ethereum::{util::ordered_trie_root, EnvelopedDecodable, EnvelopedEncodable, PartialHeader};
use ethereum_types::{Address, Bloom, H256, U256};
use ethers::types::Bytes;
use header::{Header, Seal};
use keccak_hash::keccak;
use rlp::{Decodable, DecoderError, Encodable, RlpStream, *};
use serde::{Deserialize, Serialize};
use std::{cmp, collections::HashSet, sync::Arc};

#[derive(Debug, Copy, Clone)]
pub enum VerifyBlockError {
    EmptyInput = -3001,
    PrevBlockHashWrong = -3002,
    CurrentBlockHashWrong = -3003,
    ParentHashWrong = -3004,
    BlockNumbersNotConsistent = -3005,
}

impl Into<i32> for VerifyBlockError {
    fn into(self) -> i32 {
        self as i32
    }
}

/// Helper structure, used for encoding blocks.
#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Block {
    pub header: Header,
    pub transactions: Vec<transaction::Transaction>,
    pub uncles: Vec<Header>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct BlockX<T> {
    pub header: Header,
    pub transactions: Vec<T>,
    pub uncles: Vec<Header>,
}

impl<T: EnvelopedEncodable> Encodable for BlockX<T> {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(3);
        s.append(&self.header);
        s.append_list::<Vec<u8>, _>(
            &self
                .transactions
                .iter()
                .map(|tx| EnvelopedEncodable::encode(tx).to_vec())
                .collect::<Vec<_>>(),
        );
    }
}

impl<T: EnvelopedDecodable> Decodable for BlockX<T> {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        Ok(Self {
            header: rlp.val_at(0)?,
            transactions: rlp
                .list_at::<Vec<u8>>(1)?
                .into_iter()
                .map(|raw_tx| {
                    EnvelopedDecodable::decode(&raw_tx)
                        .map_err(|_| DecoderError::Custom("decode enveloped transaction failed"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            uncles: rlp.list_at(2)?,
        })
    }
}

impl<T: EnvelopedEncodable> BlockX<T> {
    pub fn new(partial_header: PartialHeader, transactions: Vec<T>, uncles: Vec<Header>) -> Self {
        // let ommers_hash = H256::from_slice(keccak(&rlp::encode_list(&uncles)[..]).as_bytes());
        // let transactions_root = ordered_trie_root(
        //     transactions
        //         .iter()
        //         .map(|r| EnvelopedEncodable::encode(r).freeze()),
        // );

        Self {
            header: Header::new(),
            transactions,
            uncles,
        }
    }
}

impl Block {
    /// Get the RLP-encoding of the block with or without the seal.
    pub fn rlp_bytes(&self, seal: Seal) -> Bytes {
        let mut block_rlp = RlpStream::new_list(3);
        self.header.stream_rlp(&mut block_rlp);
        block_rlp.append_list(&self.transactions);
        block_rlp.append_list(&self.uncles);
        Bytes(block_rlp.out().into())
    }

    pub fn is_equal_to(&self, cmp_block: &Block) -> bool {
        // 1. encode the self block into RLP format
        let self_block_encoded = rlp::encode(self);
        // 2. encode the comparison block into RLP format
        let cmp_block_encoded = rlp::encode(cmp_block);
        // 3. compare
        if self_block_encoded.eq(&cmp_block_encoded) {
            return false;
        }
        true
    }
}

impl Encodable for Block {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(3);
        s.append(&self.header);
        s.append_list(&self.transactions);
        s.append_list(&self.uncles);
    }
}

impl Decodable for Block {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.as_raw().len() != rlp.payload_info()?.total() {
            return Err(DecoderError::RlpIsTooBig);
        }
        if rlp.item_count()? != 3 {
            return Err(DecoderError::RlpIncorrectListLen);
        }
        Ok(Block {
            header: rlp.val_at(0)?,
            transactions: rlp.list_at(1)?,
            uncles: rlp.list_at(2)?,
        })
    }
}

// TODO
pub(crate) fn verify_input_blocks(
    prev_blk: &Block,
    cur_blk: &Block,
) -> Result<bool, VerifyBlockError> {
    // // 1. verify the parent block hash
    // if !prev_blk_hash
    //     .as_bytes()
    //     .eq(prev_blk.header.hash().as_bytes())
    // {
    //     return Err(VerifyBlockError::PrevBlockHashWrong);
    // }
    // // 2. verify the current block hash
    // if !cur_blk_hash.as_bytes().eq(cur_blk.header.hash().as_bytes()) {
    //     return Err(VerifyBlockError::CurrentBlockHashWrong);
    // }
    // 3. compare the prev block hash vs curret block's parent hash
    if !prev_blk.header.hash().eq(cur_blk.header.parent_hash()) {
        return Err(VerifyBlockError::ParentHashWrong);
    }
    // 4. verify consistency of block numbers
    if !(prev_blk.header.number() == (cur_blk.header.number() - 1)) {
        return Err(VerifyBlockError::BlockNumbersNotConsistent.into());
    }
    return Ok(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth_types::{
        header::{generate_random_header, generate_random_header_based_on_prev_block},
        *,
    };
    use ethereum_types::{Address, H256};
    use rlp;

    #[test]
    fn verify_block_inputs_with_proper_data() {
        // 1. prev block
        let (prev_blk_header, prev_blk_hash) = generate_random_header(&123120);
        let prev_blk = Block {
            header: prev_blk_header,
            transactions: vec![],
            uncles: vec![],
        };

        // 1. prev block
        let cur_blk_header = generate_random_header_based_on_prev_block(&123121, prev_blk_hash);
        let cur_blk = Block {
            header: cur_blk_header,
            transactions: vec![],
            uncles: vec![],
        };

        let res = verify_input_blocks(&prev_blk, &cur_blk);
        match res {
            Ok(res) => {
                assert!(res);
            }
            Err(err) => {
                println!("{:?}", err);
            }
        }
    }

    #[test]
    fn verify_block_inputs_with_wrong_prev_blk_num() {
        // 1. prev block
        let (prev_blk_header, prev_blk_hash) = generate_random_header(&123120);
        let prev_blk = Block {
            header: prev_blk_header,
            transactions: vec![],
            uncles: vec![],
        };

        // 2. current block
        let cur_blk_header = generate_random_header_based_on_prev_block(&123122, prev_blk_hash);
        let cur_blk = Block {
            header: cur_blk_header,
            transactions: vec![],
            uncles: vec![],
        };

        let res = verify_input_blocks(&prev_blk, &cur_blk);
        match res {
            Ok(result) => {
                assert!(!result);
            }
            Err(err) => {
                //  assert_eq!(err, VerifyBlockError::BlockNumbersNotConsistent)
            }
        }
    }

    #[test]
    fn verify_block_inputs_with_wrong_prev_blk_hash() {
        // 1. prev block
        let (prev_blk_header, _) = generate_random_header(&123120);
        let prev_blk = Block {
            header: prev_blk_header,
            transactions: vec![],
            uncles: vec![],
        };

        // 2. current block
        let cur_blk_header = generate_random_header_based_on_prev_block(&123121, H256::random());
        let cur_blk = Block {
            header: cur_blk_header,
            transactions: vec![],
            uncles: vec![],
        };

        let res = verify_input_blocks(&prev_blk, &cur_blk);
        match res {
            Ok(result) => {
                assert!(!result);
            }
            Err(err) => {
                // assert_eq!(err, VerifyBlockError::ParentHashWrong)
            }
        }
    }
}
