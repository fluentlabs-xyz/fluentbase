use alloc::vec;
use auto_impl::auto_impl;
use fluentbase_types::{Address, Bytes, B256, U256};

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
    fn meta(&self) -> &Bytes;
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

#[derive(Default, Clone, Debug, PartialEq)]
pub struct BlockContextV1 {
    pub chain_id: u64,
    pub coinbase: Address,
    pub timestamp: u64,
    pub number: u64,
    pub difficulty: U256,
    pub prev_randao: B256,
    pub gas_limit: u64,
    pub base_fee: U256,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ContractContextV1 {
    pub address: Address,
    pub bytecode_address: Address,
    pub caller: Address,
    pub is_static: bool,
    pub value: U256,
    pub gas_limit: u64,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct TxContextV1 {
    pub gas_limit: u64,
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_priority_fee: Option<U256>,
    pub origin: Address,
    // pub blob_hashes: Vec<B256>,
    // pub max_fee_per_blob_gas: Option<U256>,
    pub value: U256,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct SharedContextInputV1 {
    pub block: BlockContextV1,
    pub tx: TxContextV1,
    pub contract: ContractContextV1,
    pub meta: Option<Bytes>,
    pub is_ownable: bool,
}

impl ContextReader for SharedContextInputV1 {
    fn block_chain_id(&self) -> u64 {
        self.block.chain_id
    }

    fn block_coinbase(&self) -> Address {
        self.block.coinbase
    }

    fn block_timestamp(&self) -> u64 {
        self.block.timestamp
    }

    fn block_number(&self) -> u64 {
        self.block.number
    }

    fn block_difficulty(&self) -> U256 {
        self.block.difficulty
    }

    fn block_prev_randao(&self) -> B256 {
        self.block.prev_randao
    }

    fn block_gas_limit(&self) -> u64 {
        self.block.gas_limit
    }

    fn block_base_fee(&self) -> U256 {
        self.block.base_fee
    }

    fn tx_gas_limit(&self) -> u64 {
        self.tx.gas_limit
    }

    fn tx_nonce(&self) -> u64 {
        self.tx.nonce
    }

    fn tx_gas_price(&self) -> U256 {
        self.tx.gas_price
    }

    fn tx_gas_priority_fee(&self) -> Option<U256> {
        self.tx.gas_priority_fee
    }

    fn tx_origin(&self) -> Address {
        self.tx.origin
    }

    fn tx_value(&self) -> U256 {
        self.tx.value
    }

    fn contract_address(&self) -> Address {
        self.contract.address
    }

    fn contract_bytecode_address(&self) -> Address {
        self.contract.bytecode_address
    }

    fn meta(&self) -> &Bytes {
        self.meta.as_ref().unwrap_or_default()
    }

    fn contract_caller(&self) -> Address {
        self.contract.caller
    }

    fn contract_is_static(&self) -> bool {
        self.contract.is_static
    }

    fn contract_value(&self) -> U256 {
        self.contract.value
    }

    fn contract_gas_limit(&self) -> u64 {
        self.contract.gas_limit
    }
}

impl bincode::Encode for SharedContextInputV1 {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        e: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        let block_chain_id: u64 = self.block.chain_id;
        let block_coinbase: [u8; 20] = self.block.coinbase.into();
        let block_timestamp: u64 = self.block.timestamp;
        let block_number: u64 = self.block.number;
        let block_difficulty: [u8; 32] = self.block.difficulty.to_be_bytes();
        let block_prev_randao: [u8; 32] = self.block.prev_randao.into();
        let block_gas_limit_block: u64 = self.block.gas_limit;
        let block_base_fee: [u8; 32] = self.block.base_fee.to_be_bytes();

        let tx_gas_limit: u64 = self.tx.gas_limit;
        let tx_nonce: u64 = self.tx.nonce;
        let tx_gas_price: [u8; 32] = self.tx.gas_price.to_be_bytes();
        let tx_priority_fee_present = self.tx.gas_priority_fee.is_some();
        let tx_priority_fee: [u8; 32] = self
            .tx
            .gas_priority_fee
            .map(|v| v.to_be_bytes())
            .unwrap_or_default();
        let tx_origin: [u8; 20] = self.tx.origin.into();
        let tx_value: [u8; 32] = self.tx.value.to_be_bytes();

        let contract_address: [u8; 20] = self.contract.address.into();
        let contract_bytecode_address: [u8; 20] = self.contract.bytecode_address.into();
        let contract_caller: [u8; 20] = self.contract.caller.into();
        let contract_is_static: bool = self.contract.is_static;
        let contract_value: [u8; 32] = self.contract.value.to_be_bytes();
        let contract_gas_limit: u64 = self.contract.gas_limit;

        bincode::Encode::encode(&block_chain_id, e)?;
        bincode::Encode::encode(&block_coinbase, e)?;
        bincode::Encode::encode(&block_timestamp, e)?;
        bincode::Encode::encode(&block_number, e)?;
        bincode::Encode::encode(&block_difficulty, e)?;
        bincode::Encode::encode(&block_prev_randao, e)?;
        bincode::Encode::encode(&block_gas_limit_block, e)?;
        bincode::Encode::encode(&block_base_fee, e)?;
        bincode::Encode::encode(&tx_gas_limit, e)?;
        bincode::Encode::encode(&tx_nonce, e)?;
        bincode::Encode::encode(&tx_gas_price, e)?;
        bincode::Encode::encode(&tx_priority_fee_present, e)?;
        bincode::Encode::encode(&tx_priority_fee, e)?;
        bincode::Encode::encode(&tx_origin, e)?;
        bincode::Encode::encode(&tx_value, e)?;
        bincode::Encode::encode(&contract_address, e)?;
        bincode::Encode::encode(&contract_bytecode_address, e)?;
        bincode::Encode::encode(&contract_caller, e)?;
        bincode::Encode::encode(&contract_is_static, e)?;
        bincode::Encode::encode(&contract_value, e)?;
        bincode::Encode::encode(&contract_gas_limit, e)?;

        bincode::Encode::encode(&(self.meta.as_ref().unwrap_or_default().len() as u32), e)?;
        bincode::Encode::encode(&self.is_ownable, e)?;

        let reserved = [0u8; Self::SIZE_RESERVED];
        bincode::Encode::encode(&reserved, e)?;

        if self.meta.is_some() {
            let meta_bytes = self.meta.as_ref().unwrap_or_default();
            bincode::Encode::encode(meta_bytes.as_ref(), e)?;
        }

        Ok(())
    }
}

impl<C> bincode::Decode<C> for SharedContextInputV1 {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        d: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let block_chain_id = bincode::Decode::decode(d)?;
        let block_coinbase: [u8; 20] = bincode::Decode::decode(d)?;
        let block_timestamp = bincode::Decode::decode(d)?;
        let block_number = bincode::Decode::decode(d)?;
        let block_difficulty: [u8; 32] = bincode::Decode::decode(d)?;
        let block_prev_randao: [u8; 32] = bincode::Decode::decode(d)?;
        let block_gas_limit = bincode::Decode::decode(d)?;
        let block_base_fee: [u8; 32] = bincode::Decode::decode(d)?;

        let tx_gas_limit = bincode::Decode::decode(d)?;
        let tx_nonce = bincode::Decode::decode(d)?;
        let tx_gas_price: [u8; 32] = bincode::Decode::decode(d)?;
        let tx_gas_priority_fee_present: bool = bincode::Decode::decode(d)?;
        let tx_gas_priority_fee: [u8; 32] = bincode::Decode::decode(d)?;
        let tx_origin: [u8; 20] = bincode::Decode::decode(d)?;
        let tx_value: [u8; 32] = bincode::Decode::decode(d)?;

        let contract_address: [u8; 20] = bincode::Decode::decode(d)?;
        let contract_bytecode_address: [u8; 20] = bincode::Decode::decode(d)?;
        let contract_caller: [u8; 20] = bincode::Decode::decode(d)?;
        let contract_is_static = bincode::Decode::decode(d)?;
        let contract_value: [u8; 32] = bincode::Decode::decode(d)?;
        let contract_gas_limit = bincode::Decode::decode(d)?;

        let meta_byte_size: u32 = bincode::Decode::decode(d)?;
        let is_ownable: bool = bincode::Decode::decode(d)?;

        let _reserved: [u8; Self::SIZE_RESERVED] = bincode::Decode::decode(d)?;

        let meta: Option<Bytes> = if meta_byte_size > 0 {
            let mut meta_bytes = vec![];
            meta_bytes = bincode::Decode::decode(d)?;
            Some(meta_bytes.into())
        } else {
            None
        };

        Ok(Self {
            block: BlockContextV1 {
                chain_id: block_chain_id,
                coinbase: Address::from(block_coinbase),
                timestamp: block_timestamp,
                number: block_number,
                difficulty: U256::from_be_bytes(block_difficulty),
                prev_randao: B256::from(block_prev_randao),
                gas_limit: block_gas_limit,
                base_fee: U256::from_be_bytes(block_base_fee),
            },
            tx: TxContextV1 {
                gas_limit: tx_gas_limit,
                nonce: tx_nonce,
                gas_price: U256::from_be_bytes(tx_gas_price),
                gas_priority_fee: tx_gas_priority_fee_present
                    .then(|| U256::from_be_bytes(tx_gas_priority_fee)),
                origin: Address::from(tx_origin),
                value: U256::from_be_bytes(tx_value),
            },
            contract: ContractContextV1 {
                address: Address::from(contract_address),
                bytecode_address: Address::from(contract_bytecode_address),
                caller: Address::from(contract_caller),
                is_static: contract_is_static,
                value: U256::from_be_bytes(contract_value),
                gas_limit: contract_gas_limit,
            },
            meta,
            is_ownable,
        })
    }
}

impl SharedContextInputV1 {
    pub const SIZE: usize = 1024; // total size of encoded struct
    pub const SIZE_RESERVED: usize = 637; // size reserved for new fields

    pub fn decode_type_from_slice<T: bincode::de::Decode<()>>(
        buf: &[u8],
    ) -> Result<T, bincode::error::DecodeError> {
        let (result, _) = bincode::decode_from_slice(buf, bincode::config::legacy())?;
        Ok(result)
    }

    pub fn decode_from_slice(buf: &[u8]) -> Result<Self, bincode::error::DecodeError> {
        let result = Self::decode_type_from_slice(buf)?;
        Ok(result)
    }

    pub fn encode_to_vec(&self) -> Result<Bytes, bincode::error::EncodeError> {
        let result: Bytes = bincode::encode_to_vec(self, bincode::config::legacy())?.into();
        Ok(result)
    }
    pub fn meta_bytes_len_position() -> usize {
        382
    }
    pub fn meta_bytes_len_try_decode<BUF: AsRef<[u8]>>(
        buf: BUF,
    ) -> Result<u32, bincode::error::DecodeError> {
        Self::decode_type_from_slice(buf.as_ref())
    }
    pub fn try_decode_meta_bytes_only<BUF: AsRef<[u8]>>(
        buf: BUF,
    ) -> Result<u32, bincode::error::DecodeError> {
        Self::decode_type_from_slice(&buf.as_ref()[Self::meta_bytes_len_position()..])
    }
    pub fn compute_meta_bytes_encoded_size(len: u32) -> u32 {
        if len > 0 {
            return len + 8; // bincode encoding for metadata of Bytes or Vec<u8>
        }
        0
    }
    pub fn meta_bytes_encoded_size(&self) -> u32 {
        Self::compute_meta_bytes_encoded_size(self.meta.as_ref().unwrap_or_default().len() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    #[test]
    fn test_size_is_correct() {
        assert_eq!(SharedContextInputV1::SIZE, 1024);
        assert_eq!(SharedContextInputV1::SIZE_RESERVED, 637);
    }

    #[test]
    fn test_serialize_context() {
        let context = SharedContextInputV1 {
            block: BlockContextV1 {
                chain_id: 1,
                coinbase: Address::from(hex!("1000000000000000000000000000000000000001")),
                timestamp: 1_700_000_000,
                number: 18_000_000,
                difficulty: U256::from(0x02000000),
                prev_randao: B256::from(hex!(
                    "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                )),
                gas_limit: 30_000_000,
                base_fee: U256::from(100_000_000_000u64),
            },
            tx: TxContextV1 {
                gas_limit: 21_000,
                nonce: 42,
                gas_price: U256::from(100_000_000_000u64),
                gas_priority_fee: Some(U256::from(2_000_000_000u64)),
                origin: Address::from(hex!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef")),
                value: U256::from(1_000_000_000_000_000_000u64),
            },
            contract: ContractContextV1 {
                address: Address::from(hex!("2000000000000000000000000000000000000002")),
                bytecode_address: Address::from(hex!("3000000000000000000000000000000000000003")),
                caller: Address::from(hex!("4000000000000000000000000000000000000004")),
                is_static: false,
                value: U256::from(0),
                gas_limit: 100_000,
            },
            meta: Some(vec![1, 2, 3, 4].into()),
            is_ownable: true,
        };
        let encoded = context.encode_to_vec().unwrap();
        let meta_bytes_len = SharedContextInputV1::try_decode_meta_bytes_only(&encoded).unwrap();
        assert_eq!(meta_bytes_len, context.meta.as_ref().unwrap().len() as u32);
        let decoded = SharedContextInputV1::decode_from_slice(&encoded).unwrap();
        assert_eq!(context, decoded);
        assert_eq!(
            encoded.len(),
            SharedContextInputV1::SIZE + context.meta_bytes_encoded_size() as usize
        );
    }

    #[test]
    fn test_serialize_default_context() {
        let context = SharedContextInputV1::default();
        let encoded = context.encode_to_vec().unwrap();
        let decoded = SharedContextInputV1::decode_from_slice(&encoded).unwrap();
        assert_eq!(context, decoded);
        assert_eq!(encoded.len(), SharedContextInputV1::SIZE);
    }
}
