pub use self::{
    block_context::BlockContextV1,
    contract_context::ContractContextV1,
    tx_context::TxContextV1,
};
use crate::{
    context::{BlockContextReader, ContractContextReader, SharedContextReader, TxContextReader},
    Address,
    B256,
    U256,
};
use fluentbase_codec::Codec;

mod block_context;
mod contract_context;
mod tx_context;

#[derive(Codec, Default, Clone)]
pub struct SharedContextInputV1 {
    pub block: BlockContextV1,
    pub tx: TxContextV1,
    pub contract: ContractContextV1,
}

impl BlockContextReader for SharedContextInputV1 {
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
}

impl TxContextReader for SharedContextInputV1 {
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
}

impl ContractContextReader for SharedContextInputV1 {
    fn contract_address(&self) -> Address {
        self.contract.address
    }

    fn contract_bytecode_address(&self) -> Address {
        self.contract.bytecode_address
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
}

impl SharedContextReader for SharedContextInputV1 {}
