use bytes::BytesMut;
use ethereum::{EnvelopedDecodable, EnvelopedDecoderError, EnvelopedEncodable};
// use ethereum::Log;
use super::bytes::{de_hex_to_vec_u8, se_hex, Bytes};
use ethereum_types::{Address, Bloom, H256, U256, U64};
use rlp::*;
use serde::{Deserialize, Serialize};

// Type              uint8  `json:"type,omitempty"`
// PostState         []byte `json:"root"`
// Status            uint64 `json:"status"`
// CumulativeGasUsed uint64 `json:"cumulativeGasUsed" gencodec:"required"`
// Bloom             Bloom  `json:"logsBloom"         gencodec:"required"`
// Logs              []*Log `json:"logs"              gencodec:"required"`
// #[serde(rename_all = "camelCase")]
#[derive(
    Clone, Debug, PartialEq, Eq, rlp::RlpEncodable, rlp::RlpDecodable, Serialize, Deserialize,
)]
#[cfg_attr(
    feature = "with-codec",
    derive(codec::Encode, codec::Decode, scale_info::TypeInfo)
)]
// #[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Log {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(serialize_with = "se_hex")]
    #[serde(deserialize_with = "de_hex_to_vec_u8")]
    data: Vec<u8>,
}

#[derive(
    Clone, Debug, PartialEq, Eq, rlp::RlpEncodable, rlp::RlpDecodable, Serialize, Deserialize,
)]
#[cfg_attr(
    feature = "with-codec",
    derive(codec::Encode, codec::Decode, scale_info::TypeInfo)
)]
pub struct Receipt {
    pub status: U64,
    #[serde(rename = "cumulativeGasUsed")]
    pub cumulative_gas_used: U64,
    #[serde(rename = "logsBloom")]
    pub bloom: Bloom,
    pub logs: Vec<Log>,
}

impl EnvelopedEncodable for Receipt {
    fn type_id(&self) -> Option<u8> {
        None
    }
    fn encode_payload(&self) -> BytesMut {
        rlp::encode(self)
    }
}

impl EnvelopedDecodable for Receipt {
    type PayloadDecoderError = DecoderError;

    fn decode(bytes: &[u8]) -> Result<Self, EnvelopedDecoderError<Self::PayloadDecoderError>> {
        Ok(rlp::decode(bytes)?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ReceiptX {
    pub status: U64,
    #[serde(rename = "cumulativeGasUsed")]
    pub cumulative_gas_used: U64,
    #[serde(rename = "logsBloom")]
    pub bloom: Bloom,
    pub logs: Vec<Log>,
}

impl Receipt {}

// impl Encodable for Receipt {
//     fn rlp_append(&self, stream: &mut RlpStream) {
//         stream.begin_list(4);
//         stream.append(&self.status);
//         stream.append(&self.cumulative_gas_used);
//         stream.append(&self.bloom);
//         stream.append_list(&self.logs);
//     }
// }

// impl Decodable for Receipt {
//     fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
//         let result = Receipt {
//             status: rlp.val_at(0)?,
//             cumulative_gas_used: rlp.val_at(1)?,
//             bloom: rlp.val_at(2)?,
//             logs: rlp.list_at(3)?,
//         };
//         Ok(result)
//     }
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
