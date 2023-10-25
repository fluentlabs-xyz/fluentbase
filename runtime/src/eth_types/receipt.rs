use ethereum_types::{Address, Bloom, H256, U256};
use ethers::types::Log;
use rlp::*;

/// EVM log's receipt.
#[derive(Clone, Debug, Default)]
pub struct Receipt {
    pub id: usize,
    pub status: u8,
    pub cumulative_gas_used: u64,
    pub bloom: Bloom,
    pub logs: Vec<Log>,
}

impl Receipt {
    pub fn new(
        state_root: Option<H256>,
        quota_used: U256,
        logs: Vec<Log>,
        account_nonce: U256,
        transaction_hash: H256,
    ) -> Receipt {
        Receipt {
            //     bloom: logs.iter().fold(Bloom::default(), |b, l| b | l()),
            id: todo!(),
            status: todo!(),
            cumulative_gas_used: todo!(),
            bloom: todo!(),
            logs,
        }
    }
}

impl Encodable for Receipt {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(4);
        s.append(&self.status);
        s.append(&self.cumulative_gas_used);
        s.append(&self.bloom);
        s.begin_list(self.logs.len());
        for log in self.logs.iter() {
            s.begin_list(3);
            s.append(&log.address);
            s.append_list(&log.topics);
            s.append(&log.data.0);
        }
    }
}

// impl Decodable for Receipt {
//     // fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
//     //     if rlp.as_raw().len() != rlp.payload_info()?.total() {
//     //         return Err(DecoderError::RlpIsTooBig);
//     //     }

//     //     // if rlp.item_count()? != 5 {
//     //     //     return Err(DecoderError::RlpIncorrectListLen);
//     //     // }
//     //     // Ok(Receipt {
//     //     //     status: rlp.val_at(0)?,
//     //     //     cumulative_gas_used: rlp.val_at(1)?,
//     //     //     bloom: rlp.val_at(2)?,
//     //     //     logs: rlp.list_at(3),
//     //     //     id: todo!(),
//     //     // })
//     // }
// }

#[cfg(test)]
mod tests {
    use super::*;
    //use crate::log::Log;

    // #[test]
    // fn test_no_state_root() {
    //     let r = Receipt::new(
    //         None,
    //         0x40cae.into(),
    //         vec![Log {
    //             address: "dcf421d093428b096ca501a7cd1a740855a7976f".into(),
    //             topics: vec![],
    //             data: //vec![0u8; 32],
    //             block_number: todo!(),
    //             transaction_hash: todo!(),
    //             transaction_index: todo!(),
    //             log_index: todo!(),
    //             transaction_log_index: todo!(),
    //             log_type: todo!(),
    //             removed: todo!(),
    //         }],
    //         None,
    //         1.into(),
    //         "2f697d671e9ae4ee24a43c4b0d7e15f1cb4ba6de1561120d43b9a4e8c4a8a6ee".into(),
    //     );
    //     let encoded = ::rlp::encode(&r);
    //     println!("encode ok");
    //     let decoded: Receipt = ::rlp::decode(&encoded);
    //     println!("decoded: {:?}", decoded);
    //     assert_eq!(decoded, r);
    // }
}
