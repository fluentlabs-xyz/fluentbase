use crate::{
    alloc_slice,
    Account,
    AccountCheckpoint,
    LowLevelSDK,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
};
use alloc::{vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_codec_derive::Codec;
use fluentbase_types::{Address, Bytes, Bytes32, B256, U256};

pub trait ContextReader {
    fn block_chain_id(&self) -> u64;
    fn block_coinbase(&self) -> Address;
    fn block_timestamp(&self) -> u64;
    fn block_number(&self) -> u64;
    fn block_difficulty(&self) -> u64;
    fn block_prevrandao(&self) -> B256;
    fn block_gas_limit(&self) -> u64;
    fn block_base_fee(&self) -> U256;
    fn tx_gas_limit(&self) -> u64;
    fn tx_nonce(&self) -> u64;
    fn tx_gas_price(&self) -> U256;
    fn tx_caller(&self) -> Address;
    fn tx_access_list(&self) -> Vec<(Address, Vec<U256>)>;
    fn tx_gas_priority_fee(&self) -> Option<U256>;
    fn tx_blob_hashes(&self) -> Vec<B256>;
    fn tx_blob_hashes_size(&self) -> (u32, u32);
    fn tx_max_fee_per_blob_gas(&self) -> Option<U256>;
    fn contract_gas_limit(&self) -> u64;
    fn contract_address(&self) -> Address;
    fn contract_caller(&self) -> Address;
    fn contract_value(&self) -> U256;
    fn contract_is_static(&self) -> bool;
}

#[derive(Clone, Debug, Default, Codec)]
pub struct ContractInput {
    // block info
    pub block_chain_id: u64,
    pub block_coinbase: Address,
    pub block_timestamp: u64,
    pub block_number: u64,
    pub block_difficulty: u64,
    pub block_prevrandao: B256,
    pub block_gas_limit: u64,
    pub block_base_fee: U256,
    // tx info
    pub tx_gas_limit: u64,
    pub tx_nonce: u64,
    pub tx_gas_price: U256,
    pub tx_gas_priority_fee: Option<U256>,
    pub tx_caller: Address,
    pub tx_access_list: Vec<(Address, Vec<U256>)>,
    pub tx_blob_hashes: Vec<B256>,
    pub tx_max_fee_per_blob_gas: Option<U256>,
    // contract info
    pub contract_gas_limit: u64,
    pub contract_address: Address,
    pub contract_caller: Address,
    pub contract_value: U256,
    pub contract_is_static: bool,
}

impl ContractInput {
    pub fn clone_from_cr<CR: ContextReader>(cr: &CR) -> ContractInput {
        ContractInput {
            block_chain_id: cr.block_chain_id(),
            block_coinbase: cr.block_coinbase(),
            block_timestamp: cr.block_timestamp(),
            block_number: cr.block_number(),
            block_difficulty: cr.block_difficulty(),
            block_prevrandao: cr.block_prevrandao(),
            block_gas_limit: cr.block_gas_limit(),
            block_base_fee: cr.block_base_fee(),
            tx_gas_limit: cr.tx_gas_limit(),
            tx_nonce: cr.tx_nonce(),
            tx_gas_price: cr.tx_gas_price(),
            tx_gas_priority_fee: cr.tx_gas_priority_fee(),
            tx_caller: cr.tx_caller(),
            tx_access_list: cr.tx_access_list(),
            tx_blob_hashes: cr.tx_blob_hashes(),
            tx_max_fee_per_blob_gas: cr.tx_max_fee_per_blob_gas(),
            contract_gas_limit: cr.contract_gas_limit(),
            contract_address: cr.contract_address(),
            contract_caller: cr.contract_caller(),
            contract_value: cr.contract_value(),
            contract_is_static: cr.contract_is_static(),
        }
    }
}

impl ContextReader for ContractInput {
    fn block_chain_id(&self) -> u64 {
        self.block_chain_id
    }

    fn block_coinbase(&self) -> Address {
        self.block_coinbase
    }

    fn block_timestamp(&self) -> u64 {
        self.block_timestamp
    }

    fn block_number(&self) -> u64 {
        self.block_number
    }

    fn block_difficulty(&self) -> u64 {
        self.block_difficulty
    }

    fn block_prevrandao(&self) -> B256 {
        self.block_prevrandao
    }

    fn block_gas_limit(&self) -> u64 {
        self.block_gas_limit
    }

    fn block_base_fee(&self) -> U256 {
        self.block_base_fee
    }

    fn tx_gas_limit(&self) -> u64 {
        self.tx_gas_limit
    }

    fn tx_nonce(&self) -> u64 {
        self.tx_nonce
    }

    fn tx_gas_price(&self) -> U256 {
        self.tx_gas_price
    }

    fn tx_caller(&self) -> Address {
        self.tx_caller
    }

    fn tx_access_list(&self) -> Vec<(Address, Vec<U256>)> {
        self.tx_access_list.clone()
    }

    fn tx_gas_priority_fee(&self) -> Option<U256> {
        self.tx_gas_priority_fee
    }

    fn tx_blob_hashes(&self) -> Vec<B256> {
        self.tx_blob_hashes.clone()
    }

    fn tx_blob_hashes_size(&self) -> (u32, u32) {
        (0, self.tx_blob_hashes.len() as u32 * 32)
    }

    fn tx_max_fee_per_blob_gas(&self) -> Option<U256> {
        self.tx_max_fee_per_blob_gas.clone()
    }

    fn contract_gas_limit(&self) -> u64 {
        self.contract_gas_limit
    }

    fn contract_address(&self) -> Address {
        self.contract_address
    }

    fn contract_caller(&self) -> Address {
        self.contract_caller
    }

    fn contract_value(&self) -> U256 {
        self.contract_value
    }

    fn contract_is_static(&self) -> bool {
        self.contract_is_static
    }
}
