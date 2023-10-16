// use super::bloom::WithBloom;
use crate::eth_t::{header, transaction};
use ethereum_types::{Address, Bloom, H256, U256};
use header::{Header, Seal};
use rlp::{Decodable, DecoderError, Encodable, RlpStream, *};
use std::{cmp, collections::HashSet, sync::Arc};

pub type Bytes = Vec<u8>;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VerifyBlockError {
    BlockHashWrong,

    PrevBlockHashWrong,

    CurrentBlockHashWrong,

    ParentHashWrong,

    BlockNumbersNotConsistent,

    /// Custom rlp decoding error.
    Custom(&'static str),
}

/// Helper structure, used for encoding blocks.
#[derive(Default)]
pub struct Block {
    pub header: Header,
    pub transactions: Vec<transaction::UnverifiedTransaction>,
    pub uncles: Vec<Header>,
}

impl Encodable for Block {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(3);
        s.append(&self.header);
        s.append_list(&self.transactions);
        s.append_list(&self.uncles);
    }
}

impl Block {
    /// Get the RLP-encoding of the block with or without the seal.
    pub fn rlp_bytes(&self, seal: Seal) -> Bytes {
        let mut block_rlp = RlpStream::new_list(3);
        self.header.stream_rlp(&mut block_rlp, seal);
        block_rlp.append_list(&self.transactions);
        block_rlp.append_list(&self.uncles);
        block_rlp.out().to_vec()
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

        //
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

pub fn verify_input_blocks(
    prev_blk_hash: &[u8],
    cur_blk_hash: &[u8],
    prev_blk: &Block,
    cur_blk: &Block,
) -> Result<bool, VerifyBlockError> {
    if !prev_blk_hash.eq(prev_blk.header.hash().as_bytes()) {
        return Err(VerifyBlockError::BlockHashWrong);
    }
    if !cur_blk_hash.eq(cur_blk.header.hash().as_bytes()) {
        return Err(VerifyBlockError::BlockHashWrong);
    }
    //
    if !prev_blk.is_equal_to(cur_blk) {
        return Err(VerifyBlockError::PrevBlockHashWrong);
    }
    //
    if !prev_blk.header.hash().eq(cur_blk.header.parent_hash()) {
        return Err(VerifyBlockError::ParentHashWrong);
    }
    return Ok(true);
}

mod tests {
    use crate::eth_t::{block::Block, header::Header};
    use ethereum_types::{Address, Bloom, H256};
    #[test]
    fn test_encode_block_header() {
        let expected = hex::decode("f901f9a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000940000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008208ae820d0582115c8215b3821a0a827788a00000000000000000000000000000000000000000000000000000000000000000880000000000000000").unwrap();
        let mut data = vec![];
        let header = Header {
            parent_hash: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            // beneficiary: H160::from_str("0000000000000000000000000000000000000000").unwrap(),
            state_root: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            transactions_root: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            receipts_root: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            // logs_bloom: <[u8;
            // 256]>::from_hex("
            //
            // ).unwrap().into(),
            difficulty: 0x8aeu64.into(),
            number: 0xd05u64.into(),
            gas_limit: 0x115cu64.into(),
            gas_used: 0x15b3u64.into(),
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            author: ethereum_types::H160(""),
            uncles_hash:  H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            log_bloom: Bloom::from_slice( "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000".as_bytes()),
            seal: todo!(),
            hash: todo!(),
            bare_hash: todo!(),
            // mix_hash: H256::from_str(
            //     "0000000000000000000000000000000000000000000000000000000000000000",
            // )
            //  .unwrap(),
            //nonce: H64::from_low_u64_be(0x0),
            // base_fee_per_gas: None,
        };
        // TODO
        // header.encode(&mut data);
        assert_eq!(hex::encode(&data), hex::encode(expected));
        //  assert_eq!(header.length(), data.len());
    }

    #[test]
    fn test_decode_block_header() {
        let rlp_hex =
    hex::decode("
    f901f9a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000940000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008208ae820d0582115c8215b3821a0a827788a00000000000000000000000000000000000000000000000000000000000000000880000000000000000"
    ).unwrap();
        let expected = Header {
            parent_hash: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            author: Address::from_slice("0000000000000000000000000000000000000000".as_bytes()),
            state_root: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            transactions_root: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            receipts_root: H256::from_slice(
                "0000000000000000000000000000000000000000000000000000000000000000".as_bytes(),
            ),
            //   log_bloom:
            // Bloom::from_slice("
            // 00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            // .as_bytes()),
            difficulty: 0x8aeu64.into(),
            number: 0xd05u64.into(),
            gas_limit: 0x115cu64.into(),
            gas_used: 0x15b3u64.into(),
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            uncles_hash: todo!(),
            seal: todo!(),
            hash: todo!(),
            bare_hash: todo!(),
            log_bloom: todo!(),
        };
        let header = rlp::decode::<crate::eth_t::header::Header>(&rlp_hex).unwrap();
        assert_eq!(header, expected);
    }
}
