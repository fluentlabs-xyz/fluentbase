use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rlp::{Decodable, Encodable, Error, RlpDecodable, RlpEncodable};

#[derive(Debug, Clone, RlpEncodable, RlpDecodable)]
pub struct ContractInput {
    pub input: Bytes,
    pub bytecode: Bytes,
    pub hash: B256,
    pub address: Address,
    pub caller: Address,
    pub value: U256,
}

impl ContractInput {
    pub fn read_from_slice(mut input: &[u8]) -> Result<Self, Error> {
        Self::decode(&mut input)
    }
}
