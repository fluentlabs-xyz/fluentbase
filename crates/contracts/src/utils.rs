use fluentbase_sdk::{
    codec::Encoder,
    types::{CoreInput, ICoreInput},
    Address,
    SovereignAPI,
    U256,
};
use zeth_primitives::transactions::ethereum::{EthereumTxEssence, TransactionKind};

#[allow(dead_code)]
pub(crate) fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
    let mut core_input = T::default();
    <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
    core_input
}

// pub fn evm_builder_apply_envs<'a, SDK: SovereignAPI, BuilderStage, EXT, DB: Database>(
//     builder: EvmBuilder<'a, BuilderStage, EXT, DB>,
//     sdk: &'a SDK,
// ) -> EvmBuilder<'a, BuilderStage, EXT, DB> {
//     builder
//         // .with_db(block_builder.db.take().unwrap())
//         .with_spec_id(SpecId::CANCUN)
//         .modify_block_env(|blk_env| {
//             blk_env.number = U256::from(sdk.block_context().number);
//             blk_env.coinbase = sdk.block_context().coinbase;
//             blk_env.timestamp = U256::from(sdk.block_context().timestamp);
//             blk_env.difficulty = U256::from(sdk.block_context().difficulty);
//             blk_env.prevrandao = Some(sdk.block_context().prev_randao);
//             blk_env.basefee = sdk.block_context().base_fee;
//             blk_env.gas_limit = U256::from(sdk.block_context().gas_limit);
//         })
//         .modify_cfg_env(|cfg_env| {
//             cfg_env.chain_id = sdk.block_context().chain_id;
//         })
// }
//
// pub fn fill_eth_tx_env(tx_env: &mut TxEnv, essence: &EthereumTxEssence, caller: Address) {
//     match essence {
//         EthereumTxEssence::Legacy(tx) => {
//             tx_env.caller = caller;
//             tx_env.gas_limit = tx.gas_limit.try_into().unwrap();
//             tx_env.gas_price = tx.gas_price;
//             tx_env.gas_priority_fee = None;
//             tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
//                 TransactTo::Call(to_addr)
//             } else {
//                 TransactTo::create()
//             };
//             tx_env.value = tx.value;
//             tx_env.data = tx.data.clone();
//             tx_env.chain_id = tx.chain_id;
//             tx_env.nonce = Some(tx.nonce);
//             tx_env.access_list.clear();
//         }
//         EthereumTxEssence::Eip2930(tx) => {
//             tx_env.caller = caller;
//             tx_env.gas_limit = tx.gas_limit.try_into().unwrap();
//             tx_env.gas_price = tx.gas_price;
//             tx_env.gas_priority_fee = None;
//             tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
//                 TransactTo::Call(to_addr)
//             } else {
//                 TransactTo::create()
//             };
//             tx_env.value = tx.value;
//             tx_env.data = tx.data.clone();
//             tx_env.chain_id = Some(tx.chain_id);
//             tx_env.nonce = Some(tx.nonce);
//             tx_env.access_list = tx.access_list.clone().into();
//         }
//         EthereumTxEssence::Eip1559(tx) => {
//             tx_env.caller = caller;
//             tx_env.gas_limit = tx.gas_limit.try_into().unwrap();
//             tx_env.gas_price = tx.max_fee_per_gas;
//             tx_env.gas_priority_fee = Some(tx.max_priority_fee_per_gas);
//             tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
//                 TransactTo::Call(to_addr)
//             } else {
//                 TransactTo::create()
//             };
//             tx_env.value = tx.value;
//             tx_env.data = tx.data.clone();
//             tx_env.chain_id = Some(tx.chain_id);
//             tx_env.nonce = Some(tx.nonce);
//             tx_env.access_list = tx
//                 .access_list
//                 .clone()
//                 .0
//                 .into_iter()
//                 .map(|item| item.into())
//                 .collect();
//         }
//     };
// }
