use crate::context::TxContextReader;
use alloy_primitives::{Address, U256};

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

impl TxContextReader for TxContextV1 {
    fn tx_gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn tx_nonce(&self) -> u64 {
        self.nonce
    }

    fn tx_gas_price(&self) -> U256 {
        self.gas_price
    }

    fn tx_gas_priority_fee(&self) -> Option<U256> {
        self.gas_priority_fee
    }

    fn tx_origin(&self) -> Address {
        self.origin
    }

    fn tx_value(&self) -> U256 {
        self.value
    }
}

// impl From<&primitives::Env> for TxContextV1 {
//     fn from(value: &primitives::Env) -> Self {
//         Self {
//             gas_limit: value.tx.gas_limit,
//             nonce: value.tx.nonce.unwrap_or_default(),
//             gas_price: value.tx.gas_price,
//             gas_priority_fee: value.tx.gas_priority_fee,
//             origin: value.tx.caller,
//             // data: value.tx.data.clone(),
//             // blob_hashes: value.tx.blob_hashes.clone(),
//             // max_fee_per_blob_gas: value.tx.max_fee_per_blob_gas,
//             value: value.tx.value,
//         }
//     }
// }
