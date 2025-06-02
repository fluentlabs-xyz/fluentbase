use crate::context::BlockContextReader;
use alloy_primitives::{Address, B256, U256};

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

impl BlockContextReader for BlockContextV1 {
    fn block_chain_id(&self) -> u64 {
        self.chain_id
    }

    fn block_coinbase(&self) -> Address {
        self.coinbase
    }

    fn block_timestamp(&self) -> u64 {
        self.timestamp
    }

    fn block_number(&self) -> u64 {
        self.number
    }

    fn block_difficulty(&self) -> U256 {
        self.difficulty
    }

    fn block_prev_randao(&self) -> B256 {
        self.prev_randao
    }

    fn block_gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn block_base_fee(&self) -> U256 {
        self.base_fee
    }
}

// impl From<&primitives::Env> for BlockContextV1 {
//     fn from(value: &primitives::Env) -> Self {
//         Self {
//             chain_id: value.cfg.chain_id,
//             coinbase: value.block.coinbase,
//             timestamp: value.block.timestamp.as_limbs()[0],
//             number: value.block.number.as_limbs()[0],
//             difficulty: value.block.difficulty,
//             prev_randao: value.block.prevrandao.unwrap_or_default(),
//             gas_limit: value.block.gas_limit.as_limbs()[0],
//             base_fee: value.block.basefee,
//         }
//     }
// }
