use crate::{Address, B256, U256};
use fluentbase_types::{ContextReader, SharedContextInputV1};

pub struct ContextReaderImpl<'a>(pub &'a SharedContextInputV1);

impl<'a> ContextReader for ContextReaderImpl<'a> {
    fn block_chain_id(&self) -> u64 {
        self.0.block.chain_id
    }

    fn block_coinbase(&self) -> Address {
        self.0.block.coinbase
    }

    fn block_timestamp(&self) -> u64 {
        self.0.block.timestamp
    }

    fn block_number(&self) -> u64 {
        self.0.block.number
    }

    fn block_difficulty(&self) -> U256 {
        self.0.block.difficulty
    }

    fn block_prev_randao(&self) -> B256 {
        self.0.block.prev_randao
    }

    fn block_gas_limit(&self) -> u64 {
        self.0.block.gas_limit
    }

    fn block_base_fee(&self) -> U256 {
        self.0.block.base_fee
    }

    fn tx_gas_limit(&self) -> u64 {
        self.0.tx.gas_limit
    }

    fn tx_nonce(&self) -> u64 {
        self.0.tx.nonce
    }

    fn tx_gas_price(&self) -> U256 {
        self.0.tx.gas_price
    }

    fn tx_gas_priority_fee(&self) -> Option<U256> {
        self.0.tx.gas_priority_fee
    }

    fn tx_origin(&self) -> Address {
        self.0.tx.origin
    }

    fn tx_value(&self) -> U256 {
        self.0.tx.value
    }

    fn contract_address(&self) -> Address {
        self.0.contract.address
    }

    fn contract_bytecode_address(&self) -> Address {
        self.0.contract.bytecode_address
    }

    fn contract_caller(&self) -> Address {
        self.0.contract.caller
    }

    fn contract_is_static(&self) -> bool {
        self.0.contract.is_static
    }

    fn contract_value(&self) -> U256 {
        self.0.contract.value
    }

    fn contract_gas_limit(&self) -> u64 {
        self.0.contract.gas_limit
    }
}
