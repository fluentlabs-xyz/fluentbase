use crate::eth_types::transaction::Transaction;
use ethereum_types::U64;
use ethers_core::types::{
    transaction::eip2718::TypedTransaction,
    Eip1559TransactionRequest,
    Eip2930TransactionRequest,
    NameOrAddress,
    TransactionRequest,
    H256,
};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use strum_macros::EnumIter;

/// Tx type
#[derive(Default, Debug, Copy, Clone, EnumIter, Serialize, Deserialize, PartialEq, Eq)]
pub enum TxType {
    /// EIP 155 tx
    #[default]
    Eip155 = 0,
    /// Pre EIP 155 tx
    PreEip155,
    /// EIP 1559 tx
    Eip1559,
    /// EIP 2930 tx
    Eip2930,
}

impl From<TxType> for usize {
    fn from(value: TxType) -> Self {
        value as usize
    }
}

impl From<TxType> for u64 {
    fn from(value: TxType) -> Self {
        value as u64
    }
}

impl TxType {
    /// If this type is Eip155 or not
    pub fn is_eip155_tx(&self) -> bool {
        matches!(*self, TxType::Eip155)
    }

    /// Return the recovery id of signature for recovering the signing pk
    pub fn get_recovery_id(&self, v: u64) -> u8 {
        let recovery_id = match *self {
            TxType::Eip155 => (v + 1) % 2,
            TxType::PreEip155 => {
                assert!(v == 0x1b || v == 0x1c, "v: {v}");
                v - 27
            }
            TxType::Eip1559 => {
                assert!(v <= 1);
                v
            }
            TxType::Eip2930 => {
                assert!(v <= 1);
                v
            }
        };

        recovery_id as u8
    }
}
