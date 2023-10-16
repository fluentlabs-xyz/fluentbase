// use crate::Bytes;
// extern crate alloc;
// use alloc::vec::Vec;
// use ethereum_types::{H160, H256};

// #[derive(Clone, Debug, PartialEq, Eq, rlp::RlpEncodable, rlp::RlpDecodable)]
// #[cfg_attr(
//     feature = "with-codec",
//     derive(codec::Encode, codec::Decode, scale_info::TypeInfo)
// )]
// #[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
// pub struct Log {
//     pub address: H160,
//     pub topics: Vec<H256>,
//     pub data: Bytes,
// }
