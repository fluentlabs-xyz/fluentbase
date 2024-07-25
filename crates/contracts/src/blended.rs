use crate::{
    alloc::vec::Vec,
    utils::{evm_builder_apply_envs, fill_eth_tx_env, fuel_testnet_consensus_params},
};
use alloy_rlp::{Decodable, Encodable};
use fluentbase_core::fvm::transact::_fvm_transact_inner;
use fluentbase_sdk::{
    basic_entrypoint,
    contracts::BlendedAPI,
    derive::{derive_keccak256_id, Contract},
    AccountManager,
    Bytes,
    ContextReader,
    LowLevelSDK,
    SharedAPI,
    SovereignAPI,
    U256,
};
use fuel_core_types::{
    blockchain::{
        header::{ApplicationHeader, ConsensusHeader, PartialBlockHeader},
        primitives::{DaBlockHeight, Empty},
    },
    fuel_types::ContractId,
    tai64::Tai64,
};
use fuel_vm::{
    checked_transaction::IntoChecked,
    fuel_types,
    fuel_types::{
        canonical::{Deserialize, Serialize},
        BlockHeight,
    },
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
        let tx: fuel_tx::Transaction = fuel_tx::Transaction::from_bytes(&raw_fuel_tx.as_ref())
            .expect("failed to decode transaction");
        let consensus_params = fuel_testnet_consensus_params(self.cr);
        let tx_gas_price = self.cr.tx_gas_price().as_limbs()[0];
        let coinbase_contract_id = ContractId::zeroed();
        let header = PartialBlockHeader {
            application: ApplicationHeader {
                da_height: DaBlockHeight::default(),
                consensus_parameters_version: 1,
                state_transition_bytecode_version: 1,
                generated: Empty::default(),
            },
            consensus: ConsensusHeader {
                prev_root: Default::default(),
                height: BlockHeight::new(self.cr.block_number() as u32),
                time: Tai64::UNIX_EPOCH,
                generated: Empty::default(),
            },
        };
        let receipts = match tx {
            fuel_tx::Transaction::Script(etx) => {
                let checked_tx = etx
                    .into_checked(
                        BlockHeight::new(self.cr.block_number() as u32),
                        &consensus_params,
                    )
                    .expect("convert into checked");
                let res = _fvm_transact_inner(
                    self.cr,
                    self.am,
                    checked_tx,
                    &header,
                    coinbase_contract_id,
                    tx_gas_price,
                    consensus_params,
                )
                .expect("fvm transact inner success");
                res.3.to_vec()
            }
            fuel_tx::Transaction::Create(etx) => {
                let checked_tx = etx
                    .into_checked(
                        BlockHeight::new(self.cr.block_number() as u32),
                        &consensus_params,
                    )
                    .expect("failed to convert tx into checked tx");
                let res = _fvm_transact_inner(
                    self.cr,
                    self.am,
                    checked_tx,
                    &header,
                    coinbase_contract_id,
                    tx_gas_price,
                    consensus_params,
                )
                .expect("fvm transact inner success");
                res.3.to_vec()
            }
            fuel_tx::Transaction::Upgrade(etx) => {
                let checked_tx = etx
                    .into_checked(
                        BlockHeight::new(self.cr.block_number() as u32),
                        &consensus_params,
                    )
                    .expect("failed to convert tx into checked tx");
                let res = _fvm_transact_inner(
                    self.cr,
                    self.am,
                    checked_tx,
                    &header,
                    coinbase_contract_id,
                    tx_gas_price,
                    consensus_params,
                )
                .expect("fvm transact inner success");
                res.3.to_vec()
            }
            fuel_tx::Transaction::Upload(etx) => {
                let checked_tx = etx
                    .into_checked(
                        BlockHeight::new(self.cr.block_number() as u32),
                        &consensus_params,
                    )
                    .expect("failed to convert tx into checked tx");
                let res = _fvm_transact_inner(
                    self.cr,
                    self.am,
                    checked_tx,
                    &header,
                    coinbase_contract_id,
                    tx_gas_price,
                    consensus_params,
                )
                .expect("fvm transact inner success");
                res.3.to_vec()
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
                // vm_result.receipts().clone().to_vec()
                // vec![]
            }
            fuel_tx::Transaction::Mint(_) => {
                panic!("mint transaction not supported")
            }
        };
        for receipt in &receipts {
            match receipt {
                fuel_tx::Receipt::Call {
                    id,
                    to,
                    amount,
                    asset_id,
                    gas,
                    param1,
                    param2,
                    pc,
                    is,
                } => {
                    let id = derive_keccak256_id!(
                        "Call(bytes32,uint64,bytes32,uint64,uint64,uint64,uint64,uint64)"
                    );
                    // LowLevelSDK::emit_log(id.to_be_bytes()[12..].as_ptr(), [id,] as )
                }
                fuel_tx::Receipt::Return { .. } => {}
                fuel_tx::Receipt::ReturnData { .. } => {}
                fuel_tx::Receipt::Panic { .. } => {}
                fuel_tx::Receipt::Revert { .. } => {}
                fuel_tx::Receipt::Log { .. } => {}
                fuel_tx::Receipt::LogData { .. } => {}
                fuel_tx::Receipt::Transfer { .. } => {}
                fuel_tx::Receipt::TransferOut { .. } => {}
                fuel_tx::Receipt::ScriptResult { .. } => {}
                fuel_tx::Receipt::MessageOut { .. } => {}
                fuel_tx::Receipt::Mint { .. } => {}
                fuel_tx::Receipt::Burn { .. } => {}
            }
        }
        let mut receipts_encoded = Vec::<u8>::new();
        receipts
            .encode(&mut receipts_encoded)
            .expect("failed to encode receipts");
        LowLevelSDK::write(receipts_encoded.as_ptr(), receipts_encoded.len() as u32);
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
