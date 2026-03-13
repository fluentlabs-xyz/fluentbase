use super::utils::recover_address;
use crate::runner::{TestError, TestErrorKind};
use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_sdk::{Address, PRECOMPILE_EVM_RUNTIME};
use revm::{
    bytecode::{ownable_account::OwnableAccountBytecode, Bytecode},
    context::{BlockEnv, CfgEnv, TransactTo, TransactionType::Eip1559, TxEnv},
    database::CacheState,
    primitives::{keccak256, B256, U256},
    state::AccountInfo,
};
use revm_statetest_types::{Test, TestUnit, TransactionParts};
use std::{sync::Arc, time::Instant};

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
    let mut cache_state = CacheState::new(false);
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
    let mut cache_state = CacheState::new(false);

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
            // set account info bytecode to the proxy loader
            let mut metadata = vec![];
            metadata.extend_from_slice(evm_code_hash.as_slice());
            metadata.extend_from_slice(info.code.as_ref());
            let bytecode = Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                PRECOMPILE_EVM_RUNTIME,
                metadata.into(),
            ));
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
    name: &String,
) -> Result<(CfgEnv, BlockEnv, TxEnv), TestError> {
    let mut cfg_env = CfgEnv::default();
    let mut block_env = BlockEnv::default();
    let mut tx_env = TxEnv::default();

    // for mainnet
    cfg_env.chain_id = 1;

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
            name: name.clone(),
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

    tx_env.tx_type = Eip1559 as u8;
    tx_env.nonce = transaction.nonce.to();
}
