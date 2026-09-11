use super::utils::recover_address;
use crate::runner::{TestError, TestErrorKind};
use alloy_consensus::{transaction::SignerRecoverable, Transaction, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_sdk::{Address, PRECOMPILE_EVM_RUNTIME};
use revm::{
    bytecode::Bytecode,
    context::{either::Either, BlockEnv, CfgEnv, TransactTo, TransactionType, TxEnv},
    database::CacheState,
    primitives::{keccak256, B256, U256},
    state::AccountInfo,
};
use revm_statetest_types::{Test, TestUnit, TransactionParts};
use std::{sync::Arc, time::Instant};

/// State tests can include invalid signed envelopes that cannot reach an EVM.
/// Apply the same Alloy decoding and low-S signature recovery used by admission,
/// rather than trusting the fixture's sender or ignoring its raw transaction.
pub(crate) fn signed_transaction_sender_and_chain_id(
    bytes: &[u8],
) -> Result<(Address, Option<u64>), String> {
    let mut remaining = bytes;
    let tx = TxEnvelope::decode_2718(&mut remaining).map_err(|error| error.to_string())?;
    if !remaining.is_empty() {
        return Err("trailing bytes after signed transaction".to_owned());
    }
    let sender = tx.recover_signer().map_err(|error| error.to_string())?;
    Ok((sender, tx.chain_id()))
}

thread_local! {
    pub static GENESIS_CONTRACTS: Arc<Vec<(Address, B256, Bytecode)>> = {
        let mut genesis_contracts = vec![];
        for (address, genesis_account) in GENESIS_CONTRACTS_BY_ADDRESS.iter() {
            let bytecode = Bytecode::new_raw(genesis_account.rwasm_bytecode.clone());
            genesis_contracts.push((*address, genesis_account.rwasm_bytecode_hash, bytecode));
        }
        Arc::new(genesis_contracts)
    };
}

pub(crate) fn evm_cache_state(unit: &TestUnit) -> CacheState {
    let mut cache_state = CacheState::new();
    for (address, info) in &unit.pre {
        let acc_info = AccountInfo {
            balance: info.balance,
            code_hash: keccak256(&info.code),
            nonce: info.nonce,
            code: Some(Bytecode::new_raw(info.code.clone())),
            ..Default::default()
        };
        cache_state.insert_account_with_storage(*address, acc_info, info.storage.clone());
    }
    cache_state
}

pub(crate) fn fluent_cache_state(unit: &TestUnit) -> CacheState {
    let mut cache_state = CacheState::new();

    if cfg!(feature = "debug-print") {
        println!("\nloading EVM accounts:");
    }
    let start = Instant::now();

    for (address, info) in &unit.pre {
        let mut acc_info = cache_state
            .accounts
            .get(address)
            .and_then(|a| a.account.clone())
            .map(|a| a.info)
            .unwrap_or_default();
        if !acc_info.balance.is_zero() && !info.balance.is_zero() {
            assert_eq!(
                acc_info.balance, info.balance,
                "genesis account balance mismatch, this test won't work"
            );
        }
        acc_info.balance = info.balance;
        acc_info.nonce = info.nonce;
        let prev_code_len = acc_info.code.as_ref().map(|v| v.len()).unwrap_or_default();
        if prev_code_len > 0 && !info.code.is_empty() {
            println!(
                "WARN: code length collision for an account ({address}), this test might not work"
            );
        }
        let evm_code_hash = keccak256(&info.code);
        // write EVM code hash state
        if !info.code.is_empty() {
            let code = Bytecode::new_raw(info.code.clone());
            // Delegation designators belong to the host's EIP-7702 account model.
            // Wrapping one as EVM runtime metadata hides it from authorization
            // processing and incorrectly makes a delegated sender a contract.
            if code.is_eip7702() {
                acc_info.code_hash = evm_code_hash;
                acc_info.code = Some(code);
                cache_state.insert_account_with_storage(*address, acc_info, info.storage.clone());
                continue;
            }
            // set account info bytecode to the proxy loader
            let mut metadata = vec![];
            metadata.extend_from_slice(evm_code_hash.as_slice());
            metadata.extend_from_slice(info.code.as_ref());
            let bytecode = Bytecode::new_ownable_account(PRECOMPILE_EVM_RUNTIME, metadata.into());
            acc_info.code_hash = bytecode.hash_slow();
            acc_info.code = Some(bytecode);
        }
        // write evm account into state
        cache_state.insert_account_with_storage(*address, acc_info, info.storage.clone());
    }
    if cfg!(feature = "debug-print") {
        println!("loaded evm accounts in: {:?}", start.elapsed());
    }

    cache_state
}

pub(crate) fn prepare_env(
    unit: &TestUnit,
    name: &str,
) -> Result<(CfgEnv, BlockEnv, TxEnv), TestError> {
    let mut cfg_env = CfgEnv::default();
    let mut block_env = BlockEnv::default();
    let mut tx_env = TxEnv::default();

    // for mainnet
    cfg_env.chain_id = unit.env.current_chain_id.map(|id| id.to()).unwrap_or(1);

    // block env
    block_env.number = unit.env.current_number.to();
    block_env.beneficiary = unit.env.current_coinbase;
    block_env.timestamp = unit.env.current_timestamp.to();
    block_env.gas_limit = unit.env.current_gas_limit.to();
    block_env.basefee = unit.env.current_base_fee.unwrap_or_default().to();
    block_env.difficulty = unit.env.current_difficulty.to();
    // after the Merge prevrandao replaces the mix_hash field in the block and replaced difficulty
    // opcode in EVM.
    block_env.prevrandao = unit.env.current_random;

    // tx env
    tx_env.caller = if let Some(address) = unit.transaction.sender {
        address
    } else {
        recover_address(unit.transaction.secret_key.as_slice()).ok_or_else(|| TestError {
            name: name.to_owned(),
            kind: TestErrorKind::UnknownPrivateKey(unit.transaction.secret_key),
        })?
    };
    // Handle gas price overflow - if the gas price is too large for u128,
    // this should result in a GASLIMIT_PRICE_PRODUCT_OVERFLOW exception
    let gas_price_value = unit
        .transaction
        .gas_price
        .or(unit.transaction.max_fee_per_gas)
        .unwrap_or_default();

    // Check if gas price is too large to fit in u128 (causes overflow)
    if gas_price_value > U256::from(u128::MAX) {
        // This is the case where gas price is too large to fit in u128
        // This should result in GASLIMIT_PRICE_PRODUCT_OVERFLOW exception
        // We'll use the maximum u128 value and let the EVM handle the overflow
        tx_env.gas_price = u128::MAX;
    } else {
        tx_env.gas_price = gas_price_value.to();
    }
    tx_env.gas_priority_fee = unit.transaction.max_priority_fee_per_gas.map(|v| v.to());
    // EIP-4844
    tx_env.blob_hashes = unit.transaction.blob_versioned_hashes.clone();
    tx_env.max_fee_per_blob_gas = unit
        .transaction
        .max_fee_per_blob_gas
        .map(|v| v.to())
        .unwrap_or_default();

    Ok((cfg_env, block_env, tx_env))
}

pub(crate) fn fill_tx_env(tx_env: &mut TxEnv, transaction: &TransactionParts, test: &Test) {
    tx_env.gas_limit = transaction.gas_limit[test.indexes.gas].saturating_to();

    tx_env.data = transaction.data.get(test.indexes.data).unwrap().clone();
    tx_env.value = transaction.value[test.indexes.value];

    tx_env.access_list = transaction
        .access_lists
        .get(test.indexes.data)
        .and_then(Clone::clone)
        .unwrap_or_default();

    tx_env.kind = match transaction.to {
        Some(add) => TransactTo::Call(add),
        None => TransactTo::Create,
    };

    // Preserve the fixture's transaction type, including malformed blob/set-code
    // creations: those must be rejected by the engines, not skipped by the harness.
    tx_env.tx_type = transaction.tx_type(test.indexes.data).unwrap_or_else(|| {
        if transaction.max_fee_per_blob_gas.is_some() {
            TransactionType::Eip4844
        } else {
            TransactionType::Eip7702
        }
    }) as u8;
    tx_env.authorization_list = transaction
        .authorization_list
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|authorization| Either::Left(authorization.into()))
        .collect();
    tx_env.nonce = transaction.nonce.to();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signed_envelopes_recover_sender_and_chain_id_and_reject_trailing_bytes() {
        // Official low_gas_limit fixture, case g2; signature made by upstream.
        let bytes = hex::decode("02f86601018203e88203e88261a894779c79e3bfba76b7b777511e4d055730ac3871218000c080a06a34bd91ad6014bc165a2b354ca8c1a50b633d400a86069d350bbc016c0fa9a3a05edb1a7b7c68694671fa5e387f8189ba2e1c4d0c4a4629d0ac440c5636ac1b02").unwrap();
        assert_eq!(
            signed_transaction_sender_and_chain_id(&bytes).unwrap(),
            (
                "0x57bd80ea46be74523e9060337ed2dc017f1cbf2c"
                    .parse()
                    .unwrap(),
                Some(1)
            )
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(signed_transaction_sender_and_chain_id(&trailing).is_err());
        assert!(signed_transaction_sender_and_chain_id(&[]).is_err());
        // A zero S scalar is not a valid signature, even with a valid envelope.
        let mut zero_s = bytes;
        let start = zero_s.len() - 32;
        zero_s[start..].fill(0);
        assert!(signed_transaction_sender_and_chain_id(&zero_s).is_err());
    }

    #[test]
    fn fixture_transaction_types_reach_engine_validation() {
        let base = json!({
            "data": ["0x"], "gasLimit": ["0x100000"], "gasPrice": "0x0a",
            "nonce": "0x00", "value": ["0x00"], "secretKey": B256::ZERO
        });
        let test: Test = serde_json::from_value(json!({
            "hash": B256::ZERO, "logs": B256::ZERO,
            "indexes": {"data": 0, "gas": 0, "value": 0}
        }))
        .unwrap();
        // All use CREATE, including the invalid blob/set-code transactions.
        // Both execution engines must receive those invalid types and reject them.
        for (fields, expected_type) in [
            (json!({}), TransactionType::Legacy),
            (json!({"accessLists": [[]]}), TransactionType::Eip2930),
            (json!({"maxFeePerGas": "0x0a"}), TransactionType::Eip1559),
            (
                json!({"maxFeePerBlobGas": "0x01"}),
                TransactionType::Eip4844,
            ),
            (json!({"authorizationList": []}), TransactionType::Eip7702),
        ] {
            let mut value = base.clone();
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let transaction: TransactionParts = serde_json::from_value(value).unwrap();
            let mut env = TxEnv::default();
            fill_tx_env(&mut env, &transaction, &test);
            assert_eq!(env.tx_type, expected_type as u8);
            assert_eq!(env.kind, TransactTo::Create);
        }
    }
}
