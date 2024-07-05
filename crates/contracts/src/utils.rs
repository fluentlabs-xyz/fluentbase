use core::str::FromStr;
use fluentbase_sdk::{
    codec::Encoder,
    types::{CoreInput, ICoreInput},
    Address,
    ContextReader,
    U256,
};
use fuel_tx::{
    consensus_parameters::{
        ConsensusParametersV1,
        ContractParametersV1,
        FeeParametersV1,
        PredicateParametersV1,
        ScriptParametersV1,
        TxParametersV1,
    },
    ConsensusParameters,
    ContractParameters,
    FeeParameters,
    GasCosts,
    PredicateParameters,
    ScriptParameters,
    TxParameters,
};
use fuel_vm::{
    checked_transaction::IntoChecked,
    fuel_types,
    fuel_types::AssetId,
    interpreter::CheckedMetadata,
    prelude::ExecutableTransaction,
};
use revm::{
    primitives::{hex::FromHex, SpecId, TransactTo, TxEnv},
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
        // .with_db(block_builder.db.take().unwrap())
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

// pub fn fuel_exec_tx<CR: ContextReader, Tx: ExecutableTransaction + IntoChecked>(
//     cr: &CR,
//     tx: Tx,
//     interpreter_params: fuel_vm::interpreter::InterpreterParams,
//     consensus_params: &fuel_tx::ConsensusParameters,
// ) -> fuel_vm::state::StateTransition<Tx>
// where
//     <Tx as IntoChecked>::Metadata: CheckedMetadata,
// {
//     let tx_gas_price = cr.tx_gas_price().as_limbs()[0];
//     let mut vm: fuel_vm::interpreter::Interpreter<_, _, _> =
//         fuel_vm::interpreter::Interpreter::with_storage(
//             fuel_vm::interpreter::MemoryInstance::new(),
//             fuel_vm::storage::MemoryStorage::default(),
//             interpreter_params,
//         );
//     let ready_tx = tx
//         .into_checked(
//             fuel_vm::fuel_types::BlockHeight::new(cr.block_number() as u32),
//             consensus_params,
//         )
//         .expect("failed to convert tx into checked tx")
//         .into_ready(
//             tx_gas_price,
//             consensus_params.gas_costs(),
//             consensus_params.fee_params(),
//         )
//         .expect("failed to make tx ready");
//     let vm_result: fuel_vm::state::StateTransition<_> = vm
//         .transact(ready_tx)
//         .expect("failed to exec transaction")
//         .into();
//     vm_result
// }

pub fn fuel_testnet_consensus_params<CR: ContextReader>(cr: &CR) -> ConsensusParameters {
    ConsensusParameters::V1(ConsensusParametersV1 {
        tx_params: TxParameters::V1(TxParametersV1 {
            max_inputs: 255,
            max_outputs: 255,
            max_witnesses: 255,
            max_gas_per_tx: cr.tx_gas_limit(),
            max_size: 110 * 1024,
            max_bytecode_subsections: 255,
        }),
        predicate_params: PredicateParameters::V1(PredicateParametersV1 {
            max_predicate_length: 1024 * 1024,
            max_predicate_data_length: 1024 * 1024,
            max_message_data_length: 1024 * 1024,
            max_gas_per_predicate: cr.tx_gas_limit(),
        }),
        script_params: ScriptParameters::V1(ScriptParametersV1 {
            max_script_length: 1024 * 1024,
            max_script_data_length: 1024 * 1024,
        }),
        contract_params: ContractParameters::V1(ContractParametersV1 {
            contract_max_size: 100 * 1024,
            max_storage_slots: 1760,
        }),
        fee_params: FeeParameters::V1(FeeParametersV1 {
            gas_price_factor: 92,
            gas_per_byte: 62,
        }),
        chain_id: fuel_types::ChainId::new(cr.block_chain_id()),
        gas_costs: GasCosts::default(),
        base_asset_id: AssetId::from_str(
            "f8f8b6283d7fa5b672b530cbb84fcccb4ff8dc40f8176ef4544ddb1f1952ad07",
        )
        .expect("invalid asset id format"),
        block_gas_limit: cr.block_gas_limit(),
        privileged_address: fuel_types::Address::from_str(
            "9f0e19d6c2a6283a3222426ab2630d35516b1799b503f37b02105bebe1b8a3e9",
        )
        .expect("invalid privileged address format"),
    })
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
