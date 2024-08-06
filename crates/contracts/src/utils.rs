use core::str::FromStr;
use fluentbase_core::fvm::helpers::fuel_testnet_consensus_params_from;
use fluentbase_sdk::{
    codec::Encoder,
    types::{CoreInput, ICoreInput},
    Address,
    ContextReader,
    U256,
};
use fuel_core_types::{
    fuel_tx::{
        consensus_parameters::{
            ConsensusParametersV1,
            ContractParametersV1,
            FeeParametersV1,
            PredicateParametersV1,
            ScriptParametersV1,
            TxParametersV1,
        },
        AssetId,
        ConsensusParameters,
        ContractParameters,
        FeeParameters,
        GasCosts,
        PredicateParameters,
        ScriptParameters,
        TxParameters,
    },
    fuel_types,
};
use revm::{
    primitives::{SpecId, TransactTo, TxEnv},
    Database,
    EvmBuilder,
};
use zeth_primitives::transactions::ethereum::{EthereumTxEssence, TransactionKind};

#[allow(dead_code)]
pub(crate) fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
    let mut core_input = T::default();
    <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
    core_input
}

pub fn evm_builder_apply_envs<'a, CR: ContextReader, BuilderStage, EXT, DB: Database>(
    builder: EvmBuilder<'a, BuilderStage, EXT, DB>,
    cr: &'a CR,
) -> EvmBuilder<'a, BuilderStage, EXT, DB> {
    builder
        .with_spec_id(SpecId::CANCUN)
        .modify_block_env(|blk_env| {
            blk_env.number = U256::from(cr.block_number());
            blk_env.coinbase = cr.block_coinbase();
            blk_env.timestamp = U256::from(cr.block_timestamp());
            blk_env.difficulty = U256::from(cr.block_difficulty());
            blk_env.prevrandao = Some(cr.block_prevrandao());
            blk_env.basefee = cr.block_base_fee();
            blk_env.gas_limit = U256::from(cr.tx_gas_limit());
        })
        .modify_cfg_env(|cfg_env| {
            cfg_env.chain_id = cr.block_chain_id();
        })
}

pub fn fuel_testnet_consensus_params_from_cr<CR: ContextReader>(cr: &CR) -> ConsensusParameters {
    fuel_testnet_consensus_params_from(
        cr.tx_gas_limit(),
        cr.tx_gas_limit(),
        cr.block_gas_limit(),
        fuel_types::ChainId::new(cr.block_chain_id()),
        None,
    )
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
            tx_env.access_list = tx
                .access_list
                .clone()
                .0
                .into_iter()
                .map(|item| item.into())
                .collect();
        }
    };
}
