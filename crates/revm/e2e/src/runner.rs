use super::{
    merkle_trie::{log_rlp_hash, state_merkle_trie_root},
    models::{SpecName, Test, TestSuite},
    utils::recover_address,
};
use crate::merkle_trie::state_merkle_trie_root2;
use fluentbase_genesis::devnet::{devnet_genesis_from_file, KECCAK_HASH_KEY, POSEIDON_HASH_KEY};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{Address, ExitCode};
use indicatif::{ProgressBar, ProgressDrawTarget};
use lazy_static::lazy_static;
use revm::{
    db::{states::plain_account::PlainStorage, EmptyDB},
    inspector_handle_register,
    inspectors::TracerEip3155,
    interpreter::CreateScheme,
    primitives::{
        calc_excess_blob_gas,
        keccak256,
        AccountInfo,
        Bytecode,
        Bytes,
        EVMError,
        Env,
        ExecutionResult,
        SpecId,
        TransactTo,
        B256,
        KECCAK_EMPTY,
        POSEIDON_EMPTY,
        U256,
    },
    Evm,
    State,
};
use serde_json::json;
use std::{
    convert::Infallible,
    io::{stderr, stdout},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
        Mutex,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Error)]
#[error("Test {name} failed: {kind}")]
pub struct TestError {
    pub name: String,
    pub kind: TestErrorKind,
}

#[derive(Debug, Error)]
pub enum TestErrorKind {
    #[error("logs root mismatch (spec_name={spec_name:?}): expected {expected:?}, got {got:?}")]
    LogsRootMismatch {
        spec_name: SpecName,
        got: B256,
        expected: B256,
    },
    #[error("state root mismatch (spec_name={spec_name:?}): expected {expected:?}, got {got:?}")]
    StateRootMismatch {
        spec_name: SpecName,
        got: B256,
        expected: B256,
    },
    #[error("Unknown private key: {0:?}")]
    UnknownPrivateKey(B256),
    #[error("Unexpected exception (spec_name={spec_name:?}): {got_exception:?} but test expects:{expected_exception:?}")]
    UnexpectedException {
        spec_name: SpecName,
        expected_exception: Option<String>,
        got_exception: Option<String>,
    },
    #[error("Unexpected output (spec_name={spec_name:?}): {got_output:?} but test expects:{expected_output:?}")]
    UnexpectedOutput {
        spec_name: SpecName,
        expected_output: Option<Bytes>,
        got_output: Option<Bytes>,
    },
    #[error(transparent)]
    SerdeDeserialize(#[from] serde_json::Error),
}

pub fn find_all_json_tests(path: &Path) -> Vec<PathBuf> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .map(DirEntry::into_path)
        .collect::<Vec<PathBuf>>()
}

fn skip_test(path: &Path) -> bool {
    let path_str = path.to_str().expect("Path is not valid UTF-8");
    let name = path.file_name().unwrap().to_str().unwrap();

    matches!(
        name,
        // funky test with `bigint 0x00` value in json :) not possible to happen on mainnet and
        // require custom json parser. https://github.com/ethereum/tests/issues/971
        |"ValueOverflow.json"| "ValueOverflowParis.json"

        // precompiles having storage is not possible
        | "RevertPrecompiledTouch_storage.json"
        | "RevertPrecompiledTouch.json"

        // txbyte is of type 02 and we dont parse tx bytes for this test to fail.
        | "typeTwoBerlin.json"

        // Need to handle Test errors
        | "transactionIntinsicBug.json"

        // Test check if gas price overflows, we handle this correctly but does not match tests specific exception.
        | "HighGasPrice.json"
        | "CREATE_HighNonce.json"
        | "CREATE_HighNonceMinus1.json"
        | "CreateTransactionHighNonce.json"

        // Skip test where basefee/accesslist/difficulty is present but it shouldn't be supported in
        // London/Berlin/TheMerge. https://github.com/ethereum/tests/blob/5b7e1ab3ffaf026d99d20b17bb30f533a2c80c8b/GeneralStateTests/stExample/eip1559.json#L130
        // It is expected to not execute these tests.
        | "basefeeExample.json"
        | "eip1559.json"
        | "mergeTest.json"

        // These tests are passing, but they take a lot of time to execute so we are going to skip them.
        | "loopExp.json"
        | "Call50000_sha256.json"
        | "static_Call50000_sha256.json"
        | "loopMul.json"
        | "CALLBlake2f_MaxRounds.json"
    ) || path_str.contains("stEOF")
}

fn check_evm_execution<EXT1, EXT2>(
    test: &Test,
    spec_name: &SpecName,
    expected_output: Option<&Bytes>,
    test_name: &str,
    exec_result1: &Result<ExecutionResult, EVMError<Infallible>>,
    exec_result2: &Result<ExecutionResult, EVMError<ExitCode>>,
    evm: &Evm<'_, EXT1, &mut State<EmptyDB>>,
    evm2: &fluentbase_revm::Evm<
        '_,
        EXT2,
        &mut fluentbase_revm::State<fluentbase_revm::db::EmptyDBTyped<ExitCode>>,
    >,
    print_json_outcome: bool,
) -> Result<(), TestError> {
    let logs_root = log_rlp_hash(exec_result1.as_ref().map(|r| r.logs()).unwrap_or_default());
    let logs_root2 = log_rlp_hash(exec_result2.as_ref().map(|r| r.logs()).unwrap_or_default());

    let state_root = state_merkle_trie_root(evm.context.evm.db.cache.trie_account().into_iter());
    let _state_root2 =
        state_merkle_trie_root2(evm2.context.evm.db.cache.trie_account().into_iter());

    let print_json_output = |error: Option<String>| {
        if print_json_outcome {
            let json = json!({
                    "stateRoot": state_root,
                    "logsRoot": logs_root,
                    "output": exec_result1.as_ref().ok().and_then(|r| r.output().cloned()).unwrap_or_default(),
                    "gasUsed": exec_result1.as_ref().ok().map(|r| r.gas_used()).unwrap_or_default(),
                    "pass": error.is_none(),
                    "errorMsg": error.unwrap_or_default(),
                    "evmResult": exec_result1.as_ref().err().map(|e| e.to_string()).unwrap_or("Ok".to_string()),
                    "postLogsHash": logs_root,
                    "fork": evm.handler.cfg().spec_id,
                    "test": test_name,
                    "d": test.indexes.data,
                    "g": test.indexes.gas,
                    "v": test.indexes.value,
            });
            eprintln!("{json}");
        }
    };

    // if we expect exception revm should return error from execution.
    // So we do not check logs and state root.
    //
    // Note that some tests that have exception and run tests from before state clear
    // would touch the caller account and make it appear in state root calculation.
    // This is not something that we would expect as invalid tx should not touch state.
    // but as this is a cleanup of invalid tx it is not properly defined and in the end
    // it does not matter.
    // Test where this happens: `tests/GeneralStateTests/stTransactionTest/NoSrcAccountCreate.json`
    // and you can check that we have only two "hash" values for before and after state clear.
    match (&test.expect_exception, exec_result1) {
        // do nothing
        (None, Ok(result)) => {
            // check output
            let result_output = result.output();
            if let Some((expected_output, output)) = expected_output.zip(result_output) {
                if expected_output != output {
                    let kind = TestErrorKind::UnexpectedOutput {
                        spec_name: spec_name.clone(),
                        expected_output: Some(expected_output.clone()),
                        got_output: result.output().cloned(),
                    };
                    print_json_output(Some(kind.to_string()));
                    return Err(TestError {
                        name: test_name.to_string(),
                        kind,
                    });
                }
            }
        }
        // return okay, exception is expected.
        (Some(_), Err(_)) => return Ok(()),
        _ => {
            let kind = TestErrorKind::UnexpectedException {
                spec_name: spec_name.clone(),
                expected_exception: test.expect_exception.clone(),
                got_exception: exec_result1.clone().err().map(|e| e.to_string()),
            };
            print_json_output(Some(kind.to_string()));
            return Err(TestError {
                name: test_name.to_string(),
                kind,
            });
        }
    }

    if logs_root != test.logs {
        let kind = TestErrorKind::LogsRootMismatch {
            spec_name: spec_name.clone(),
            got: logs_root,
            expected: test.logs,
        };
        print_json_output(Some(kind.to_string()));
        return Err(TestError {
            name: test_name.to_string(),
            kind,
        });
    }

    if state_root.0 != test.hash.0 {
        let kind = TestErrorKind::StateRootMismatch {
            spec_name: spec_name.clone(),
            got: state_root.0.into(),
            expected: test.hash,
        };
        print_json_output(Some(kind.to_string()));
        return Err(TestError {
            name: test_name.to_string(),
            kind,
        });
    }

    if logs_root != logs_root2 {
        let logs1 = exec_result1.as_ref().map(|r| r.logs()).unwrap_or_default();
        let logs2 = exec_result2.as_ref().map(|r| r.logs()).unwrap_or_default();
        // for log in logs1 {
        //     println!(
        //         " - {}: {}",
        //         hex::encode(log.address),
        //         log.topics()
        //             .get(0)
        //             .map(|v| hex::encode(&v))
        //             .unwrap_or_default()
        //     )
        // }
        // for log in logs2 {
        //     println!(
        //         " - {}: {}",
        //         hex::encode(log.address),
        //         log.topics()
        //             .get(0)
        //             .map(|v| hex::encode(&v))
        //             .unwrap_or_default()
        //     )
        // }
        assert_eq!(
            logs1.len(),
            logs2.len(),
            "EVM <> FLUENT logs count mismatch"
        );
        assert_eq!(logs_root, logs_root2, "EVM <> FLUENT logs root mismatch");
    }

    // compare contracts
    for (k, v) in evm.context.evm.db.cache.contracts.iter() {
        let v2 = evm2
            .context
            .evm
            .db
            .cache
            .contracts
            .get(k)
            .expect("missing fluent contract");
        // we compare only evm bytecode
        assert_eq!(v.bytecode, v2.bytecode, "EVM bytecode mismatch");
    }
    for (address, v1) in evm.context.evm.db.cache.accounts.iter() {
        println!("comparing account (0x{})...", hex::encode(address));
        let v2 = evm2.context.evm.db.cache.accounts.get(address);
        if let Some(a1) = v1.account.as_ref().map(|v| &v.info) {
            let a2 = v2
                .expect("missing FLUENT account")
                .account
                .as_ref()
                .map(|v| &v.info)
                .expect("missing FLUENT account");
            // assert_eq!(
            //     format!("{:?}", v1.status),
            //     format!("{:?}", v2.unwrap().status),
            //     "EVM account status mismatch ({:?}) <> ({:?})",
            //     v1,
            //     v2.unwrap()
            // );
            // assert_eq!(a1.balance, a2.balance, "EVM account balance mismatch");
            println!(" - nonce: {}", a1.nonce);
            assert_eq!(a1.nonce, a2.nonce, "EVM <> FLUENT account nonce mismatch");
            println!(" - code_hash: {}", hex::encode(a1.code_hash));
            assert_eq!(
                a1.code_hash, a2.code_hash,
                "EVM <> FLUENT account code_hash mismatch",
            );
            assert_eq!(
                a1.code.as_ref().map(|b| b.original_bytes()),
                a2.code.as_ref().map(|b| b.original_bytes()),
                "EVM <> FLUENT account code mismatch",
            );
            println!(" - storage:");
            if let Some(s1) = v1.account.as_ref().map(|v| &v.storage) {
                for (slot, value) in s1.iter() {
                    println!(
                        " - + slot ({}) => ({})",
                        hex::encode(&slot.to_be_bytes::<32>()),
                        hex::encode(&value.to_be_bytes::<32>())
                    );
                    // let storage_key = calc_storage_key(address, slot.as_le_bytes().as_ptr());
                    // let fluent_evm_storage = evm2
                    //     .context
                    //     .evm
                    //     .db
                    //     .cache
                    //     .accounts
                    //     .get(&EVM_STORAGE_ADDRESS)
                    //     .expect("missing special EVM storage account");
                    // let value2 = fluent_evm_storage
                    //     .storage_slot(U256::from_le_bytes(storage_key))
                    //     .unwrap_or_else(|| panic!("missing storage key {}",
                    // hex::encode(storage_key)));
                    let value2 = v2
                        .expect("missing FLUENT account")
                        .account
                        .as_ref()
                        .map(|v| &v.storage)
                        .expect("missing FLUENT account")
                        .get(slot)
                        .unwrap_or_else(|| {
                            panic!(
                                "missing storage key {}",
                                hex::encode(slot.to_be_bytes::<32>())
                            )
                        });
                    assert_eq!(
                        *value,
                        *value2,
                        "EVM storage value ({}) mismatch",
                        hex::encode(&slot.to_be_bytes::<32>())
                    );
                }
            }
        }
    }

    assert_eq!(
        exec_result1.as_ref().unwrap().gas_used(),
        exec_result2.as_ref().unwrap().gas_used(),
        "EVM <> FLUENT gas used mismatch ({})",
        exec_result2.as_ref().unwrap().gas_used() as i64
            - exec_result1.as_ref().unwrap().gas_used() as i64
    );

    for (address, v1) in evm.context.evm.db.cache.accounts.iter() {
        println!("comparing balances (0x{})...", hex::encode(address));
        let v2 = evm2.context.evm.db.cache.accounts.get(address);
        if let Some(a1) = v1.account.as_ref().map(|v| &v.info) {
            let a2 = v2
                .expect("missing FLUENT account")
                .account
                .as_ref()
                .map(|v| &v.info)
                .expect("missing FLUENT account");
            println!(" - balance: {}", a1.balance);
            let balance_diff = if a1.balance > a2.balance {
                a1.balance - a2.balance
            } else {
                a2.balance - a1.balance
            };
            // yes, there is a 1 wei diff in some tests.... debug it please
            if balance_diff > U256::from(0) {
                assert_eq!(
                    a1.balance, a2.balance,
                    "EVM <> FLUENT account balance mismatch"
                );
            }
        }
    }

    print_json_output(None);

    Ok(())
}

lazy_static! {
    static ref EVM_LOADER: (Bytes, B256) = {
        let rwasm_bytecode: Bytes =
            include_bytes!("../../../contracts/assets/loader_contract.rwasm").into();
        let rwasm_hash = B256::from(poseidon_hash(&rwasm_bytecode));
        (rwasm_bytecode, rwasm_hash)
    };
}

pub fn execute_test_suite(
    path: &Path,
    elapsed: &Arc<Mutex<Duration>>,
    trace: bool,
    print_json_outcome: bool,
    test_num: Option<usize>,
) -> Result<(), TestError> {
    if skip_test(path) {
        return Ok(());
    }

    println!("Running test: {:?}", path);

    let s = std::fs::read_to_string(path).unwrap();
    let suite: TestSuite = serde_json::from_str(&s).map_err(|e| TestError {
        name: path.to_string_lossy().into_owned(),
        kind: e.into(),
    })?;

    let devnet_genesis = devnet_genesis_from_file();

    // let (rwasm_bytecode, rwasm_hash) = (*EVM_LOADER).clone();

    for (name, unit) in suite.0 {
        // Create database and insert cache
        let mut cache_state = revm::CacheState::new(false);
        let mut cache_state2 = fluentbase_revm::CacheState::new(false);

        // let mut evm_storage: PlainStorage = PlainStorage::default();
        for (address, info) in &devnet_genesis.alloc {
            let code_hash = info
                .storage
                .as_ref()
                .and_then(|storage| storage.get(&KECCAK_HASH_KEY))
                .cloned()
                .unwrap_or(KECCAK_EMPTY);
            let rwasm_code_hash = info
                .storage
                .as_ref()
                .and_then(|storage| storage.get(&POSEIDON_HASH_KEY))
                .cloned()
                .unwrap_or(POSEIDON_EMPTY);
            let acc_info = AccountInfo {
                balance: info.balance,
                nonce: info.nonce.unwrap_or_default(),
                code_hash,
                rwasm_code_hash,
                code: Some(Bytecode::new()),
                rwasm_code: Some(Bytecode::new_raw(info.code.clone().unwrap_or_default())),
            };
            let mut account_storage = PlainStorage::default();
            if let Some(storage) = info.storage.as_ref() {
                for (k, v) in storage.iter() {
                    // let storage_key = calc_storage_key(address, k.as_ptr());
                    // evm_storage.insert(U256::from_le_bytes(storage_key), (*v).into());
                    account_storage.insert(U256::from_be_bytes(k.0), U256::from_be_bytes(v.0));
                }
            }
            cache_state2.insert_account_with_storage(*address, acc_info, account_storage);
        }

        for (address, info) in unit.pre {
            let acc_info = AccountInfo {
                balance: info.balance,
                code_hash: keccak256(&info.code),
                nonce: info.nonce,
                code: Some(Bytecode::new_raw(info.code.clone())),
                ..Default::default()
            };
            cache_state.insert_account_with_storage(
                address,
                acc_info.clone(),
                info.storage.clone(),
            );
            // acc_info.rwasm_code_hash = rwasm_hash;
            // acc_info.rwasm_code = Some(Bytecode::new_raw(rwasm_bytecode.clone()));
            // for (k, v) in info.storage.iter() {
            //     let storage_key = calc_storage_key(&address, k.to_le_bytes::<32>().as_ptr());
            //     println!(
            //         "mapping EVM storage address=0x{}, slot={}, storage_key={}, value={}",
            //         hex::encode(&address),
            //         hex::encode(k.to_be_bytes::<32>().as_slice()),
            //         hex::encode(&storage_key),
            //         hex::encode(v.to_be_bytes::<32>().as_slice()),
            //     );
            //     evm_storage.insert(U256::from_le_bytes(storage_key), (*v).into());
            // }
            cache_state2.insert_account_with_storage(address, acc_info, info.storage);
        }

        // cache_state2.insert_account_with_storage(
        //     EVM_STORAGE_ADDRESS,
        //     AccountInfo {
        //         nonce: 1,
        //         ..AccountInfo::default()
        //     },
        //     evm_storage,
        // );

        let mut env = Box::<Env>::default();
        // for mainnet
        env.cfg.chain_id = 1;
        // env.cfg.spec_id is set down the road

        // block env
        env.block.number = unit.env.current_number;
        env.block.coinbase = unit.env.current_coinbase;
        env.block.timestamp = unit.env.current_timestamp;
        env.block.gas_limit = unit.env.current_gas_limit;
        env.block.basefee = unit.env.current_base_fee.unwrap_or_default();
        env.block.difficulty = unit.env.current_difficulty;
        // after the Merge prevrandao replaces mix_hash field in block and replaced difficulty
        // opcode in EVM.
        env.block.prevrandao = unit.env.current_random;
        // EIP-4844
        if let Some(current_excess_blob_gas) = unit.env.current_excess_blob_gas {
            env.block
                .set_blob_excess_gas_and_price(current_excess_blob_gas.to());
        } else if let (Some(parent_blob_gas_used), Some(parent_excess_blob_gas)) = (
            unit.env.parent_blob_gas_used,
            unit.env.parent_excess_blob_gas,
        ) {
            env.block
                .set_blob_excess_gas_and_price(calc_excess_blob_gas(
                    parent_blob_gas_used.to(),
                    parent_excess_blob_gas.to(),
                ));
        }

        // tx env
        let caller = if let Some(address) = unit.transaction.sender {
            address
        } else {
            recover_address(unit.transaction.secret_key.as_slice()).ok_or_else(|| TestError {
                name: name.clone(),
                kind: TestErrorKind::UnknownPrivateKey(unit.transaction.secret_key),
            })?
        };
        env.tx.caller = caller;

        let gas_price = unit
            .transaction
            .gas_price
            .or(unit.transaction.max_fee_per_gas)
            .unwrap_or_default();
        env.tx.gas_price = gas_price;

        let gas_priority_fee = unit.transaction.max_priority_fee_per_gas;
        env.tx.gas_priority_fee = gas_priority_fee;

        // EIP-4844
        let blob_hashes = unit.transaction.blob_versioned_hashes;
        env.tx.blob_hashes = blob_hashes.clone();

        let max_fee_per_blob_gas = unit.transaction.max_fee_per_blob_gas;
        env.tx.max_fee_per_blob_gas = max_fee_per_blob_gas;

        // post and execution
        for (spec_name, tests) in unit.post {
            if matches!(
                spec_name,
                SpecName::ByzantiumToConstantinopleAt5
                    | SpecName::Constantinople
                    | SpecName::Unknown
            ) {
                continue;
            }
            if spec_name.lt(&SpecName::Cancun) {
                continue;
            }

            let spec_id = spec_name.to_spec_id();
            let tests_count = tests.len();

            for (index, test) in tests.into_iter().enumerate() {
                if let Some(test_num) = test_num {
                    if test_num != 0 {
                        continue;
                    }
                }
                println!(
                    "\n\n\n\n\nRunning test with txdata: ({}/{}) {}",
                    index,
                    tests_count,
                    hex::encode(test.txbytes.clone().unwrap_or_default().as_ref())
                );
                env.tx.gas_limit = unit.transaction.gas_limit[test.indexes.gas].saturating_to();

                let data = unit
                    .transaction
                    .data
                    .get(test.indexes.data)
                    .unwrap()
                    .clone();
                env.tx.data = data.clone();

                let value = unit.transaction.value[test.indexes.value];
                env.tx.value = value;

                let access_list: Vec<(Address, Vec<U256>)> = unit
                    .transaction
                    .access_lists
                    .get(test.indexes.data)
                    .and_then(Option::as_deref)
                    .unwrap_or_default()
                    .iter()
                    .map(|item| {
                        (
                            item.address,
                            item.storage_keys
                                .iter()
                                .map(|key| U256::from_be_bytes(key.0))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                env.tx.access_list = access_list.clone();

                let to = match unit.transaction.to {
                    Some(add) => TransactTo::Call(add),
                    None => TransactTo::Create(CreateScheme::Create),
                };
                env.tx.transact_to = to.clone();

                let mut cache = cache_state.clone();
                cache.set_state_clear_flag(SpecId::enabled(spec_id, SpecId::SPURIOUS_DRAGON));
                let mut cache2 = cache_state2.clone();
                cache2.set_state_clear_flag(SpecId::enabled(spec_id, SpecId::SPURIOUS_DRAGON));

                let mut state = revm::db::State::builder()
                    .with_cached_prestate(cache)
                    .with_bundle_update()
                    .build();
                let mut evm = Evm::builder()
                    .with_db(&mut state)
                    .modify_env(|e| *e = env.clone())
                    .with_spec_id(spec_id)
                    .build();

                let mut state2 = fluentbase_revm::db::StateBuilder::<
                    fluentbase_revm::db::EmptyDBTyped<ExitCode>,
                >::default()
                .with_cached_prestate(cache2)
                .with_bundle_update()
                .build();
                let mut evm2 = fluentbase_revm::Evm::builder()
                    .with_db(&mut state2)
                    .modify_env(|e| *e = env.clone())
                    .with_spec_id(spec_id)
                    .build();

                // do the deed
                let (e, exec_result) = if trace {
                    let mut evm = evm
                        .modify()
                        .reset_handler_with_external_context(TracerEip3155::new(
                            Box::new(stderr()),
                            false,
                        ))
                        .append_handler_register(inspector_handle_register)
                        .build();
                    // let mut evm2 = evm2
                    //     .modify()
                    // .reset_handler_with_external_context(TracerEip3155::new(
                    //     Box::new(stderr()),
                    //     false,
                    // ))
                    // .append_handler_register(fluentbase_revm::inspector_handle_register)
                    // .build();

                    let timer = Instant::now();
                    let res = evm.transact_commit();
                    let res2 = evm2.transact_commit();
                    *elapsed.lock().unwrap() += timer.elapsed();

                    let Err(e) = check_evm_execution::<TracerEip3155, ()>(
                        &test,
                        &spec_name,
                        unit.out.as_ref(),
                        &name,
                        &res,
                        &res2,
                        &evm,
                        &evm2,
                        print_json_outcome,
                    ) else {
                        continue;
                    };
                    // reset external context
                    (e, res)
                } else {
                    let timer = Instant::now();
                    let res = evm.transact_commit();
                    let res2 = evm2.transact_commit();
                    *elapsed.lock().unwrap() += timer.elapsed();

                    // dump state and traces if test failed
                    let output = check_evm_execution::<(), ()>(
                        &test,
                        &spec_name,
                        unit.out.as_ref(),
                        &name,
                        &res,
                        &res2,
                        &evm,
                        &evm2,
                        print_json_outcome,
                    );
                    let Err(e) = output else {
                        continue;
                    };
                    (e, res)
                };

                // print only once or
                // if we are already in trace mode, just return error
                static FAILED: AtomicBool = AtomicBool::new(false);
                if FAILED.swap(true, Ordering::SeqCst) {
                    return Err(e);
                }

                // re-build to run with tracing
                let mut cache = cache_state.clone();
                cache.set_state_clear_flag(SpecId::enabled(spec_id, SpecId::SPURIOUS_DRAGON));
                let mut cache_original = cache_state2.clone();
                cache_original
                    .set_state_clear_flag(SpecId::enabled(spec_id, SpecId::SPURIOUS_DRAGON));
                let state = revm::db::State::builder()
                    .with_cached_prestate(cache)
                    .with_bundle_update()
                    .build();
                let state_original = fluentbase_revm::db::State::builder()
                    .with_cached_prestate(cache_original)
                    .with_bundle_update()
                    .build();

                let path = path.display();
                println!("\nTraces:");
                let mut evm = Evm::builder()
                    .with_spec_id(spec_id)
                    .with_db(state)
                    .with_external_context(TracerEip3155::new(Box::new(stdout()), false))
                    // .append_handler_register(inspector_handle_register)
                    .build();
                let mut evm2 = revm::Evm::builder()
                    .with_spec_id(spec_id)
                    .with_db(state_original)
                    .with_external_context(TracerEip3155::new(Box::new(stdout()), false))
                    // .append_handler_register(inspector_handle_register)
                    .build();
                let _ = evm.transact_commit();
                let _ = evm2.transact_commit();

                println!("\nExecution result: {exec_result:#?}");
                println!("\nExpected exception: {:?}", test.expect_exception);
                // println!("\nState before: {cache_state:#?}");
                // println!("\nState after: {:#?}", evm.context.evm.db.cache);
                println!("\nSpecification: {spec_id:?}");
                println!("\nEnvironment: {env:#?}");
                println!("\nTest name: {name:?} (index: {index}, path: {path}) failed:\n{e}");

                return Err(e);
            }
        }
    }
    Ok(())
}

pub fn run(
    test_files: Vec<PathBuf>,
    mut single_thread: bool,
    trace: bool,
    mut print_outcome: bool,
) -> Result<(), TestError> {
    // trace implies print_outcome
    if trace {
        print_outcome = true;
    }
    // print_outcome or trace implies single_thread
    if print_outcome {
        single_thread = true;
    }
    let n_files = test_files.len();

    let endjob = Arc::new(AtomicBool::new(false));
    let console_bar = Arc::new(ProgressBar::with_draw_target(
        Some(n_files as u64),
        ProgressDrawTarget::stdout(),
    ));
    let queue = Arc::new(Mutex::new((0usize, test_files)));
    let elapsed = Arc::new(Mutex::new(std::time::Duration::ZERO));

    let num_threads = match (single_thread, std::thread::available_parallelism()) {
        (true, _) | (false, Err(_)) => 1,
        (false, Ok(n)) => n.get(),
    };
    let num_threads = num_threads.min(n_files);
    let mut handles = Vec::with_capacity(num_threads);
    for i in 0..num_threads {
        let queue = queue.clone();
        let endjob = endjob.clone();
        let console_bar = console_bar.clone();
        let elapsed = elapsed.clone();

        let thread = std::thread::Builder::new().name(format!("runner-{i}"));

        let f = move || loop {
            if endjob.load(Ordering::SeqCst) {
                return Ok(());
            }

            let (_index, test_path) = {
                let (current_idx, queue) = &mut *queue.lock().unwrap();
                let prev_idx = *current_idx;
                let Some(test_path) = queue.get(prev_idx).cloned() else {
                    return Ok(());
                };
                *current_idx = prev_idx + 1;
                (prev_idx, test_path)
            };

            if let Err(err) = execute_test_suite(&test_path, &elapsed, trace, print_outcome, None) {
                endjob.store(true, Ordering::SeqCst);
                return Err(err);
            }
            console_bar.inc(1);
        };
        handles.push(thread.spawn(f).unwrap());
    }

    // join all threads before returning an error
    let mut errors = Vec::new();
    for handle in handles {
        if let Err(e) = handle.join().unwrap() {
            errors.push(e);
        }
    }
    console_bar.finish();

    println!(
        "Finished execution. Total CPU time: {:.6}s",
        elapsed.lock().unwrap().as_secs_f64()
    );
    if errors.is_empty() {
        println!("All tests passed!");
        Ok(())
    } else {
        let n = errors.len();
        if n > 1 {
            println!("{n} threads returned an error, out of {num_threads} total:");
            for error in &errors {
                println!("{error}");
            }
        }
        Err(errors.swap_remove(0))
    }
}
