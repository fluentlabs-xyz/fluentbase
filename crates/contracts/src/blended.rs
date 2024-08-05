use crate::{
    alloc::vec::Vec,
    utils::{evm_builder_apply_envs, fill_eth_tx_env, fuel_testnet_consensus_params},
};
use alloy_rlp::{Decodable, Encodable};
use alloy_sol_types::SolValue;
use fluentbase_core::fvm::transact::_fvm_transact_inner;
use fluentbase_sdk::{
    basic_entrypoint,
    contracts::BlendedAPI,
    derive::{derive_keccak256, Contract},
    AccountManager,
    Bytes,
    Bytes32,
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
    fuel_types::{
        canonical::{Deserialize, Serialize},
        BlockHeight,
    },
};
use revm::{
    interpreter::Host,
    primitives::{hex, ResultAndState},
    Evm,
};
use zeth_primitives::{
    receipt::Receipt,
    transactions::{ethereum::EthereumTxEssence, Transaction, TxEssence},
};

const FUEL_VM_NON_CONTRACT_LOGS_ADDRESS: Bytes32 =
    hex!("00000000000000000000000000000000000000000000000000004675656C564D"); // ANSI: FuelVM

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
                let mut checked_tx = etx
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
                    let sig = derive_keccak256!(
                        "Call(bytes32,uint64,bytes32,uint64,uint64,uint64,uint64,uint64)"
                    );
                    let log_data =
                        (to.0, amount, asset_id.0, gas, param1, param2, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Return { id, val, pc, is } => {
                    let sig = derive_keccak256!("Return(uint64,uint64,uint64,uint64)");
                    let log_data = (val, pc, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::ReturnData {
                    id,
                    ptr,
                    len,
                    digest,
                    pc,
                    is,
                    data,
                } => {
                    let sig =
                        derive_keccak256!("ReturnData(uint64,uint64,bytes32,uint64,uint64,bytes)");
                    // TODO what todo with `data` field
                    let log_data =
                        (ptr, len, digest.0, pc, is, data.clone().unwrap_or_default()).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Panic {
                    id,
                    reason,
                    pc,
                    is,
                    contract_id,
                } => {
                    // reason has 2 fields: PanicReason, RawInstruction both can be represented as
                    // (uint8,uint64)
                    let sig = derive_keccak256!("Panic(uint64,uint64,uint64,uint64,bytes32)");
                    let log_data = (
                        *reason.reason() as u64,
                        *reason.instruction() as u64,
                        pc,
                        is,
                        contract_id.unwrap_or_default().0,
                    )
                        .abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Revert { id, ra, pc, is } => {
                    let sig = derive_keccak256!("Revert(uint64,uint64,uint64)");
                    let log_data = (ra, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Log {
                    id,
                    ra,
                    rb,
                    rc,
                    rd,
                    pc,
                    is,
                } => {
                    let sig = derive_keccak256!("Log(uint64,uint64,uint64,uint64,uint64,uint64)");
                    let log_data = (ra, rb, rc, rd, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::LogData {
                    id,
                    ra,
                    rb,
                    ptr,
                    len,
                    digest,
                    pc,
                    is,
                    data,
                } => {
                    let sig = derive_keccak256!(
                        "Log(uint64,uint64,uint64,uint64,bytes32,uint64,uint64,bytes)"
                    );
                    let log_data = (
                        ra,
                        rb,
                        ptr,
                        len,
                        digest.0,
                        pc,
                        is,
                        data.clone().unwrap_or_default(),
                    )
                        .abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Transfer {
                    id,
                    to,
                    amount,
                    asset_id,
                    pc,
                    is,
                } => {
                    let sig = derive_keccak256!("Log(bytes32,uint64,bytes32,uint64,uint64)");
                    let log_data = (to.0, amount, asset_id.0, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        id[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::TransferOut {
                    id,
                    to,
                    amount,
                    asset_id,
                    pc,
                    is,
                } => {
                    let sig = derive_keccak256!("Log(bytes32,uint64,bytes32,uint64,uint64)");
                    let log_data = (to.0, amount, asset_id.0, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        FUEL_VM_NON_CONTRACT_LOGS_ADDRESS[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::ScriptResult { result, gas_used } => {
                    let sig = derive_keccak256!("ScriptResult(uint64,uint64)");
                    let result_u64: u64 = (*result).into();
                    let log_data = (result_u64, gas_used).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        FUEL_VM_NON_CONTRACT_LOGS_ADDRESS[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::MessageOut {
                    sender,
                    recipient,
                    amount,
                    nonce,
                    len,
                    digest,
                    data,
                } => {
                    let sig = derive_keccak256!(
                        "MessageOut(bytes32,bytes32,uint64,bytes32,uint64,bytes32,bytes)"
                    );
                    let log_data = (
                        sender.0,
                        recipient.0,
                        amount,
                        nonce.0,
                        len,
                        digest.0,
                        data.clone().unwrap_or_default(),
                    )
                        .abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        FUEL_VM_NON_CONTRACT_LOGS_ADDRESS[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Mint {
                    sub_id,
                    contract_id,
                    val,
                    pc,
                    is,
                } => {
                    let sig = derive_keccak256!("Mint(bytes32,bytes32,uint64,uint64,uint64)");
                    let log_data = (sub_id.0, contract_id.0, val, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        FUEL_VM_NON_CONTRACT_LOGS_ADDRESS[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
                fuel_tx::Receipt::Burn {
                    sub_id,
                    contract_id,
                    val,
                    pc,
                    is,
                } => {
                    let sig = derive_keccak256!("Burn(bytes32,bytes32,uint64,uint64,uint64)");
                    let log_data = (sub_id.0, contract_id.0, val, pc, is).abi_encode();
                    let topics = [sig];
                    LowLevelSDK::emit_log(
                        FUEL_VM_NON_CONTRACT_LOGS_ADDRESS[12..].as_ptr(),
                        topics.as_ptr(),
                        topics.len() as u32 * 32,
                        log_data.as_ptr(),
                        log_data.len() as u32,
                    );
                }
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
