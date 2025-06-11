use alloy_primitives::{Address, Bytes, B256, U256};
use auto_impl::auto_impl;

mod v1;

pub use self::v1::{BlockContextV1, ContractContextV1, SharedContextInputV1, TxContextV1};

#[auto_impl(&)]
pub trait ContextReader {
    fn block_chain_id(&self) -> u64;
    fn block_coinbase(&self) -> Address;
    fn block_timestamp(&self) -> u64;
    fn block_number(&self) -> u64;
    fn block_difficulty(&self) -> U256;
    fn block_prev_randao(&self) -> B256;
    fn block_gas_limit(&self) -> u64;
    fn block_base_fee(&self) -> U256;
    fn tx_gas_limit(&self) -> u64;
    fn tx_nonce(&self) -> u64;
    fn tx_gas_price(&self) -> U256;
    fn tx_gas_priority_fee(&self) -> Option<U256>;
    fn tx_origin(&self) -> Address;
    fn tx_value(&self) -> U256;
    fn contract_address(&self) -> Address;
    fn contract_bytecode_address(&self) -> Address;
    fn contract_caller(&self) -> Address;
    fn contract_is_static(&self) -> bool;
    fn contract_value(&self) -> U256;
    fn contract_gas_limit(&self) -> u64;
}

#[derive(Clone, Debug, PartialEq)]
pub enum SharedContextInput {
    V1(SharedContextInputV1),
}

impl SharedContextInput {
    fn version(&self) -> u8 {
        match self {
            SharedContextInput::V1(_) => 0x01,
        }
    }

    pub fn decode(buf: &[u8]) -> Result<Self, bincode::error::DecodeError> {
        let config = bincode::config::legacy();
        let result = bincode::decode_from_slice(buf, config)?;
        Ok(Self::V1(result.0))
    }

    pub fn encode(&self) -> Result<Bytes, bincode::error::EncodeError> {
        match self {
            SharedContextInput::V1(value) => {
                let config = bincode::config::legacy();
                let result: Bytes = bincode::encode_to_vec(value, config)?.into();
                Ok(result)
            }
        }
    }
}
