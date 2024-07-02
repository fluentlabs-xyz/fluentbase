use crate::{
    alloc::vec::Vec,
    utils::{evm_builder_apply_envs, fill_eth_tx_env, fuel_exec_tx},
};
use alloy_rlp::{Decodable, Encodable};
use fluentbase_sdk::{
    basic_entrypoint,
    contracts::BlendedAPI,
    derive::Contract,
    AccountManager,
    Bytes,
    ContextReader,
    LowLevelSDK,
    SharedAPI,
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
    ChargeableTransaction,
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
    fuel_types::{
        canonical::{Deserialize, Serialize},
        BlockHeight,
    },
    interpreter::ExecutableTransaction,
    prelude::{MemoryStorage, StateTransitionRef},
    transactor::Transactor,
};
use revm::{interpreter::Host, primitives::ResultAndState, Evm};
use zeth_primitives::{
    receipt::Receipt,
    transactions::{ethereum::EthereumTxEssence, Transaction, TxEssence},
};

#[derive(Contract)]
pub struct BLENDED<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> BlendedAPI for BLENDED<'a, CR, AM> {
    fn exec_evm_tx(&self, raw_evm_tx: Bytes) {
        let mut raw_tx = raw_evm_tx.clone();
        let tx = <Transaction<EthereumTxEssence> as Decodable>::decode(&mut raw_tx.as_ref())
            .expect("failed to decode transaction");
        let tx_from = tx.recover_from().expect("failed to recover tx_from");
        let mut evm = evm_builder_apply_envs(Evm::builder(), self.cr).build();
        fill_eth_tx_env(&mut evm.context.env_mut().tx, &tx.essence, tx_from);
        let ResultAndState { result, .. } = evm.transact().expect("failed to exec transaction");
        let receipt = Receipt::new(
            tx.essence.tx_type(),
            result.is_success(),
            U256::from(result.gas_used()),
            result
                .logs()
                .into_iter()
                .map(|log| log.clone().into())
                .collect(),
        );
        let mut receipt_encoded = alloy_rlp::encode(receipt);
        LowLevelSDK::write(receipt_encoded.as_ptr(), receipt_encoded.len() as u32);
    }

    fn exec_fuel_tx(&self, raw_fuel_tx: Bytes) {
        let tx = fuel_tx::Transaction::from_bytes(&raw_fuel_tx.as_ref())
            .expect("failed to decode transaction");
        let consensus_params = ConsensusParameters::V1(ConsensusParametersV1 {
            tx_params: TxParameters::V1(TxParametersV1 {
                max_inputs: 255,
                max_outputs: 255,
                max_witnesses: 255,
                max_gas_per_tx: self.cr.tx_gas_limit(),
                max_size: 110 * 1024,
                max_bytecode_subsections: 255,
            }),
            predicate_params: PredicateParameters::V1(PredicateParametersV1 {
                max_predicate_length: 1024 * 1024,
                max_predicate_data_length: 1024 * 1024,
                max_message_data_length: 1024 * 1024,
                max_gas_per_predicate: self.cr.tx_gas_limit(),
            }),
            script_params: ScriptParameters::V1(ScriptParametersV1 {
                max_script_length: 1024 * 1024,
                max_script_data_length: 1024 * 1024,
            }),
            contract_params: ContractParameters::V1(ContractParametersV1 {
                contract_max_size: 100 * 1024,
                max_storage_slots: 255,
            }),
            fee_params: FeeParameters::V1(FeeParametersV1 {
                gas_price_factor: 1_000_000_000,
                gas_per_byte: 4,
            }),
            chain_id: fuel_vm::fuel_types::ChainId::new(self.cr.block_chain_id()),
            gas_costs: GasCosts::default(),
            base_asset_id: Default::default(),
            block_gas_limit: self.cr.block_gas_limit(),
            privileged_address: Default::default(),
        });
        let tx_gas_price = self.cr.tx_gas_price().as_limbs()[0];
        let interpreter_params =
            fuel_vm::interpreter::InterpreterParams::new(tx_gas_price, &consensus_params);

        // let v = exec_fuel_tx_with_vm(self.cr, tx, interpreter_params, &consensus_params);
        let receipt = match tx {
            fuel_tx::Transaction::Script(etx) => {
                let vm_result = fuel_exec_tx(self.cr, etx, interpreter_params, &consensus_params);
                vm_result.receipts().first().cloned().expect("no receipts")
            }
            fuel_tx::Transaction::Create(etx) => {
                let vm_result = fuel_exec_tx(self.cr, etx, interpreter_params, &consensus_params);
                vm_result.receipts().first().cloned().expect("no receipts")
            }
            fuel_tx::Transaction::Upgrade(etx) => {
                let vm_result = fuel_exec_tx(self.cr, etx, interpreter_params, &consensus_params);
                vm_result.receipts().first().cloned().expect("no receipts")
            }
            fuel_tx::Transaction::Upload(etx) => {
                let vm_result = fuel_exec_tx(self.cr, etx, interpreter_params, &consensus_params);
                vm_result.receipts().first().cloned().expect("no receipts")
                // let mut vm: fuel_vm::interpreter::Interpreter<_, _, _> =
                //     fuel_vm::interpreter::Interpreter::with_storage(
                //         fuel_vm::interpreter::MemoryInstance::new(),
                //         MemoryStorage::default(),
                //         interpreter_params,
                //     );
                // let ready_tx = etx
                //     .into_checked(
                //         BlockHeight::new(self.cr.block_number() as u32),
                //         &consensus_params,
                //     )
                //     .expect("failed to convert tx into checked tx")
                //     .into_ready(
                //         tx_gas_price,
                //         consensus_params.gas_costs(),
                //         consensus_params.fee_params(),
                //     )
                //     .expect("failed to make tx ready");
                // let vm_result: fuel_vm::state::StateTransition<_> = vm
                //     .transact(ready_tx)
                //     .expect("failed to exec transaction")
                //     .into();
                // vm_result.receipts().first().cloned().expect("no receipts")
            }
            fuel_tx::Transaction::Mint(_) => {
                panic!("mint transaction is not supported")
            }
        };
        let mut receipt_encoded = Vec::<u8>::new();
        receipt
            .encode(&mut receipt_encoded)
            .expect("failed to encode receipt");
        LowLevelSDK::write(receipt_encoded.as_ptr(), receipt_encoded.len() as u32);
    }

    fn exec_svm_tx(&self, raw_svm_tx: Bytes) {
        todo!("implement svm tx")
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> BLENDED<'a, CR, AM> {
    pub fn deploy<SDK: SharedAPI>(&self) {
        unreachable!("precompiles can't be deployed, it exists since a genesis state")
    }

    pub fn main<SDK: SharedAPI>(&self) {}
}

basic_entrypoint!(
    BLENDED<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
