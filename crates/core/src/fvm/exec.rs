use crate::fvm::{
    helpers::fuel_testnet_consensus_params_from_cr,
    transact::_fvm_transact_commit_inner,
};
use alloy_sol_types::SolValue;
use fluentbase_sdk::{
    derive::derive_keccak256,
    types::FvmMethodOutput,
    Bytes,
    Bytes32,
    ExitCode,
    SharedAPI,
    B256,
};
use fuel_core_executor::executor::ExecutionData;
use fuel_core_types::{
    blockchain::{
        header::{ApplicationHeader, ConsensusHeader, PartialBlockHeader},
        primitives::{DaBlockHeight, Empty},
    },
    fuel_tx,
    fuel_types::{canonical::Deserialize, BlockHeight, ContractId},
    fuel_vm::checked_transaction::IntoChecked,
    tai64::Tai64,
};
use revm_primitives::hex;

pub const FUEL_VM_NON_CONTRACT_LOGS_ADDRESS: Bytes32 =
    hex!("00000000000000000000000000000000000000000000000000004675656C564D"); // ANSI: FuelVM

pub fn _exec_fuel_tx<SDK: SharedAPI>(
    sdk: &mut SDK,
    gas_limit: u64,
    raw_fuel_tx: Bytes,
) -> FvmMethodOutput {
    let Ok(tx) = fuel_tx::Transaction::from_bytes(&raw_fuel_tx.as_ref()) else {
        return FvmMethodOutput::from_exit_code(ExitCode::FatalExternalError)
            .with_gas(gas_limit, 0);
    };

    let consensus_params = fuel_testnet_consensus_params_from_cr(sdk);
    let tx_gas_price = sdk.tx_context().gas_price.as_limbs()[0];
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
            height: BlockHeight::new(sdk.block_context().number as u32),
            time: Tai64::UNIX_EPOCH,
            generated: Empty::default(),
        },
    };
    let mut execution_data = ExecutionData::new();
    let receipts = match tx {
        fuel_tx::Transaction::Script(etx) => {
            let checked_tx = etx
                .into_checked(
                    BlockHeight::new(sdk.block_context().number as u32),
                    &consensus_params,
                )
                .expect("convert into checked");
            let result = _fvm_transact_commit_inner(
                sdk,
                checked_tx,
                &header,
                coinbase_contract_id,
                tx_gas_price,
                consensus_params,
                &mut execution_data,
            )
            .expect("fvm transact commit inner success");
            result.receipts.to_vec()
        }
        fuel_tx::Transaction::Create(etx) => {
            let checked_tx = etx
                .into_checked(
                    BlockHeight::new(sdk.block_context().number as u32),
                    &consensus_params,
                )
                .expect("failed to convert tx into checked tx");
            let result = _fvm_transact_commit_inner(
                sdk,
                checked_tx,
                &header,
                coinbase_contract_id,
                tx_gas_price,
                consensus_params,
                &mut execution_data,
            )
            .expect("fvm transact commit inner success");
            result.receipts.to_vec()
        }
        fuel_tx::Transaction::Upgrade(etx) => {
            let checked_tx = etx
                .into_checked(
                    BlockHeight::new(sdk.block_context().number as u32),
                    &consensus_params,
                )
                .expect("failed to convert tx into checked tx");
            let res = _fvm_transact_commit_inner(
                sdk,
                checked_tx,
                &header,
                coinbase_contract_id,
                tx_gas_price,
                consensus_params,
                &mut execution_data,
            )
            .expect("fvm transact inner success");
            res.receipts.to_vec()
        }
        fuel_tx::Transaction::Upload(etx) => {
            let checked_tx = etx
                .into_checked(
                    BlockHeight::new(sdk.block_context().number as u32),
                    &consensus_params,
                )
                .expect("failed to convert tx into checked tx");
            let res = _fvm_transact_commit_inner(
                sdk,
                checked_tx,
                &header,
                coinbase_contract_id,
                tx_gas_price,
                consensus_params,
                &mut execution_data,
            )
            .expect("fvm transact inner success");
            res.receipts.to_vec()
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
                let log_data = (to.0, amount, asset_id.0, gas, param1, param2, pc, is).abi_encode();
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
            }
            fuel_tx::Receipt::Return { id, val, pc, is } => {
                let sig = derive_keccak256!("Return(uint64,uint64,uint64,uint64)");
                let log_data = (val, pc, pc, is).abi_encode();
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let log_data =
                    (ptr, len, digest.0, pc, is, data.clone().unwrap_or_default()).abi_encode();
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
            }
            fuel_tx::Receipt::Revert { id, ra, pc, is } => {
                let sig = derive_keccak256!("Revert(uint64,uint64,uint64)");
                let log_data = (ra, pc, is).abi_encode();
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig), B256::from(id.0), B256::from(to.0)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig), B256::from(id.0), B256::from(to.0)];
                sdk.emit_log(log_data.into(), &topics);
            }
            fuel_tx::Receipt::ScriptResult { result, gas_used } => {
                let sig = derive_keccak256!("ScriptResult(uint64,uint64)");
                let result_u64: u64 = (*result).into();
                let log_data = (result_u64, gas_used).abi_encode();
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
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
                let topics = [B256::from(sig)];
                sdk.emit_log(log_data.into(), &topics);
            }
        }
    }
    // let mut receipts_encoded = Vec::<u8>::new();
    // receipts
    //     .encode(&mut receipts_encoded)
    //     .expect("failed to encode receipts");
    // LowLevelSDK::write(receipts_encoded.as_ptr(), receipts_encoded.len() as u32);

    FvmMethodOutput {
        output: Default::default(),
        exit_code: ExitCode::Ok.into_i32(),
        gas_remaining: gas_limit - execution_data.used_gas(),
        gas_refund: 0,
    }
}
