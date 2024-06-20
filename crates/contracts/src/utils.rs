use fluentbase_sdk::{
    codec::Encoder,
    types::{CoreInput, ICoreInput},
    Address,
};
// use revm::primitives::{TransactTo, TxEnv};
use zeth_primitives::transactions::ethereum::{EthereumTxEssence, TransactionKind};

#[allow(dead_code)]
pub(crate) fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
    let mut core_input = T::default();
    <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
    core_input
}

pub fn fill_eth_tx_env(tx_env: &mut TxEnv, essence: &EthereumTxEssence, caller: Address) {
    match essence {
        EthereumTxEssence::Legacy(tx) => {
            tx_env.caller = caller;
            tx_env.gas_limit = tx.gas_limit.try_into().unwrap();
            tx_env.gas_price = tx.gas_price;
            tx_env.gas_priority_fee = None;
            tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
                TransactTo::Call(to_addr)
            } else {
                TransactTo::create()
            };
            tx_env.value = tx.value;
            tx_env.data = tx.data.clone();
            tx_env.chain_id = tx.chain_id;
            tx_env.nonce = Some(tx.nonce);
            tx_env.access_list.clear();
        }
        EthereumTxEssence::Eip2930(tx) => {
            tx_env.caller = caller;
            tx_env.gas_limit = tx.gas_limit.try_into().unwrap();
            tx_env.gas_price = tx.gas_price;
            tx_env.gas_priority_fee = None;
            tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
                TransactTo::Call(to_addr)
            } else {
                TransactTo::create()
            };
            tx_env.value = tx.value;
            tx_env.data = tx.data.clone();
            tx_env.chain_id = Some(tx.chain_id);
            tx_env.nonce = Some(tx.nonce);
            tx_env.access_list = tx.access_list.clone().into();
        }
        EthereumTxEssence::Eip1559(tx) => {
            tx_env.caller = caller;
            tx_env.gas_limit = tx.gas_limit.try_into().unwrap();
            tx_env.gas_price = tx.max_fee_per_gas;
            tx_env.gas_priority_fee = Some(tx.max_priority_fee_per_gas);
            tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
                TransactTo::Call(to_addr)
            } else {
                TransactTo::create()
            };
            tx_env.value = tx.value;
            tx_env.data = tx.data.clone();
            tx_env.chain_id = Some(tx.chain_id);
            tx_env.nonce = Some(tx.nonce);
            tx_env.access_list = tx.access_list.clone().into();
        }
    };
}
