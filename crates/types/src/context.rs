use crate::bytes::{Buf, BytesMut};
use alloy_primitives::{Address, Bytes, B256, U256};
use auto_impl::auto_impl;
use fluentbase_codec::{CodecError, CompactABI};

mod v1;

#[auto_impl(&)]
pub trait BlockContextReader {
    fn block_chain_id(&self) -> u64;
    fn block_coinbase(&self) -> Address;
    fn block_timestamp(&self) -> u64;
    fn block_number(&self) -> u64;
    fn block_difficulty(&self) -> U256;
    fn block_prev_randao(&self) -> B256;
    fn block_gas_limit(&self) -> u64;
    fn block_base_fee(&self) -> U256;
}

#[auto_impl(&)]
pub trait TxContextReader {
    fn tx_gas_limit(&self) -> u64;
    fn tx_nonce(&self) -> u64;
    fn tx_gas_price(&self) -> U256;
    fn tx_gas_priority_fee(&self) -> Option<U256>;
    fn tx_origin(&self) -> Address;
    fn tx_value(&self) -> U256;
}

#[auto_impl(&)]
pub trait ContractContextReader {
    fn contract_address(&self) -> Address;
    fn contract_bytecode_address(&self) -> Address;
    fn contract_caller(&self) -> Address;
    fn contract_is_static(&self) -> bool;
    fn contract_value(&self) -> U256;
    fn contract_gas_limit(&self) -> u64;
}

pub use self::v1::{BlockContextV1, ContractContextV1, SharedContextInputV1, TxContextV1};

#[auto_impl(&)]
pub trait SharedContextReader:
    BlockContextReader + TxContextReader + ContractContextReader
{
}

pub enum SharedContextInput {
    V1(SharedContextInputV1),
}

impl SharedContextInput {
    fn version(&self) -> u8 {
        match self {
            SharedContextInput::V1(_) => 0x01,
        }
    }

    pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
        // let version = buf.chunk()[0];
        // Ok(match version {
        //     0x01 => Self::V1(CompactABI::<SharedContextInputV1>::decode(buf, 1)?),
        //     _ => unreachable!("unexpected version"),
        // })
        Ok(Self::V1(CompactABI::<SharedContextInputV1>::decode(
            buf, 0,
        )?))
    }

    pub fn encode(&self) -> Result<Bytes, CodecError> {
        let mut buf = BytesMut::new();
        // buf.put_u8(self.version());
        match self {
            SharedContextInput::V1(value) => {
                CompactABI::encode(value, &mut buf, 0)?;
            }
        }
        Ok(buf.freeze().into())
    }
}
