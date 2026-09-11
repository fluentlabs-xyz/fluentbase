use super::merkle_trie::{
    compute_test_roots, log_rlp_hash, state_merkle_trie_root, TestValidationResult,
};
use crate::{
    exclusions::excluded_case_reason,
    inspector::TraceInspector,
    state::{
        evm_cache_state, fill_tx_env, fluent_cache_state, prepare_env,
        signed_transaction_sender_and_chain_id, GENESIS_CONTRACTS,
    },
};
use fluentbase_revm::{RwasmBuilder, RwasmContext, RwasmEvm, RwasmPrecompiles};
use fluentbase_sdk::{Address, PRECOMPILE_EVM_RUNTIME};
use indicatif::{ProgressBar, ProgressDrawTarget};
use revm::{
    bytecode::Bytecode,
    context::{
        result::{EVMError, ExecutionResult, HaltReason, InvalidTransaction, Output},
        ContextTr,
    },
    database::{bal::EvmDatabaseError, EmptyDB, InMemoryDB, State, StateBuilder},
    handler::{EthPrecompiles, MainnetContext},
    interpreter::InstructionResult,
    primitives::{hardfork::SpecId, Bytes, B256, U256},
    state::AccountInfo,
    Database, ExecuteCommitEvm, InspectCommitEvm, MainBuilder, MainnetEvm,
};
use revm_statetest_types::{SpecName, Test, TestSuite};
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
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
    #[error("no transactions executed (skipped {skipped} post cases); this differential suite requires Osaka fixtures to match the deployed EVM runtime")]
    NoTransactionsExecuted { skipped: usize },
    #[error("unsupported fixture fork: {0}")]
    UnsupportedFork(String),
    #[error("logs root mismatch: got {got}, expected {expected}")]
    LogsRootMismatch { got: B256, expected: B256 },
    #[error("state root mismatch: got {got}, expected {expected}")]
    StateRootMismatch { got: B256, expected: B256 },
    #[error("unknown private key: {0:?}")]
    UnknownPrivateKey(B256),
    #[error("unexpected exception: got {got_exception:?}, expected {expected_exception:?}")]
    UnexpectedException {
        expected_exception: Option<String>,
        got_exception: Option<String>,
    },
    #[error("unexpected output: got {got_output:?}, expected {expected_output:?}")]
    UnexpectedOutput {
        expected_output: Option<Bytes>,
        got_output: Option<Bytes>,
    },
    #[error(transparent)]
    SerdeDeserialize(#[from] serde_json::Error),
    #[error("thread panicked")]
    Panic,
    #[error("missing account {address}")]
    MissingAccount { address: Address },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExecutionStats {
    pub executed: usize,
    pub rejected_before_execution: usize,
    pub skipped: usize,
}

fn eligible_post_cases(suite: &TestSuite, path: &Path) -> Result<ExecutionStats, TestError> {
    let mut stats = ExecutionStats::default();
    let mut eligible = 0;
    for unit in suite.0.values() {
        for (spec, tests) in &unit.post {
            match spec {
                SpecName::Osaka => eligible += tests.len(),
                SpecName::Unknown | SpecName::Amsterdam => {
                    return Err(TestError {
                        name: path.display().to_string(),
                        kind: TestErrorKind::UnsupportedFork(format!("{spec:?}")),
                    });
                }
                _ => stats.skipped += tests.len(),
            }
        }
    }
    if eligible == 0 {
        return Err(TestError {
            name: path.display().to_string(),
            kind: TestErrorKind::NoTransactionsExecuted {
                skipped: stats.skipped,
            },
        });
    }
    Ok(stats)
}

pub fn find_all_json_tests(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension() == Some("json".as_ref()))
            .map(DirEntry::into_path)
            .collect()
    }
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

        // txbyte is of type 02 and we don't parse tx bytes for this test to fail.
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

#[allow(clippy::too_many_arguments, clippy::single_match)]
fn check_evm_execution<ERROR: Debug + ToString + Clone + PartialEq, INSP>(
    test: &Test,
    expected_output: Option<&Bytes>,
    test_name: &str,
    exec_result1: &Result<ExecutionResult, ERROR>,
    exec_result2: &Result<ExecutionResult, ERROR>,
    evm: &mut MainnetEvm<MainnetContext<State<InMemoryDB>>, INSP>,
    evm2: &mut RwasmEvm<RwasmContext<State<InMemoryDB>>, INSP>,
    print_json_outcome: bool,
) -> Result<(), TestError> {
    if exec_result1.is_err() != exec_result2.is_err() {
        return Err(TestError {
            name: test_name.to_string(),
            kind: TestErrorKind::UnexpectedException {
                expected_exception: exec_result1.as_ref().err().map(ToString::to_string),
                got_exception: exec_result2.as_ref().err().map(ToString::to_string),
            },
        });
    }

    if cfg!(feature = "debug-print") {
        println!(
            "\nexec_result_1={:?}, exec_result_2={:?}\n",
            exec_result1, exec_result2
        );
    }
    match (exec_result1, exec_result2) {
        (
            Ok(ExecutionResult::Halt {
                reason: reason1, ..
            }),
            Ok(ExecutionResult::Halt {
                reason: reason2, ..
            }),
        ) => {
            let reason1: InstructionResult = reason1.clone().into();
            let reason2: InstructionResult = reason2.clone().into();
            use InstructionResult::*;
            match (reason1, reason2) {
                (
                    OutOfGas | MemoryOOG | MemoryLimitOOG | PrecompileOOG | InvalidOperandOOG
                    | ReentrancySentryOOG,
                    OutOfFuel,
                ) => {}
                (
                    CreateContractSizeLimit
                    | CreateContractStartingWithEF
                    | CreateInitCodeSizeLimit,
                    CreateContractSizeLimit,
                ) => {}
                (OutOfOffset, InputOutputOutOfBounds) => {}
                (StackUnderflow | StackOverflow, StackOverflow) => {}
                (
                    NotActivated | InvalidJump | OpcodeNotFound | InvalidFEOpcode,
                    // The delegated EVM maps these to NotSupportedBytecode,
                    // which the host exposes as OpcodeNotFound. Gas, output,
                    // logs and state are still compared below.
                    OpcodeNotFound | MalformedBuiltinParams,
                ) => {}
                _ => {
                    assert_eq!(reason1, reason2);
                }
            }
        }
        _ => {}
    }

    let logs_root1 = log_rlp_hash(exec_result1.as_ref().map(|r| r.logs()).unwrap_or_default());
    let logs_root2 = log_rlp_hash(exec_result2.as_ref().map(|r| r.logs()).unwrap_or_default());

    let state_root1 = state_merkle_trie_root(evm.journaled_state.database.cache.trie_account());

    let print_json_output = |error: Option<String>| {
        if print_json_outcome {
            let json = json!({
                    "stateRoot": state_root1,
                    "logsRoot": logs_root1,
                    "output": exec_result1.as_ref().ok().and_then(|r| r.output().cloned()).unwrap_or_default(),
                    "gasUsed": exec_result1.as_ref().ok().map(|r| r.tx_gas_used()).unwrap_or_default(),
                    "pass": error.is_none(),
                    "errorMsg": error.unwrap_or_default(),
                    "evmResult": exec_result1.as_ref().err().map(|e| e.to_string()).unwrap_or("Ok".to_string()),
                    "postLogsHash": logs_root1,
                    "fork": evm.ctx.cfg.spec,
                    "test": test_name,
                    "d": test.indexes.data,
                    "g": test.indexes.gas,
                    "v": test.indexes.value,
            });
            eprintln!("{json}");
        }
    };

    // If we expect exception revm should return error from execution.
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
            if let Some((expected_output, output)) = expected_output.zip(result.output()) {
                if expected_output != output {
                    let kind = TestErrorKind::UnexpectedOutput {
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

    if logs_root1 != test.logs {
        let logs1 = exec_result1.as_ref().map(|r| r.logs()).unwrap_or_default();
        let logs2 = exec_result2.as_ref().map(|r| r.logs()).unwrap_or_default();

        println!("EVM logs ({:?}):", logs1);
        println!("FLUENT logs ({:?}):", logs2);

        let kind = TestErrorKind::LogsRootMismatch {
            got: logs_root1,
            expected: test.logs,
        };
        print_json_output(Some(kind.to_string()));
        return Err(TestError {
            name: test_name.to_string(),
            kind,
        });
    }

    if state_root1 != test.hash {
        let kind = TestErrorKind::StateRootMismatch {
            expected: test.hash,
            got: state_root1,
        };
        print_json_output(Some(kind.to_string()));
        return Err(TestError {
            name: test_name.to_string(),
            kind,
        });
    }

    // print_json_output(None);
    // return Ok(());

    let mut error_list: Vec<String> = vec![];

    macro_rules! error_eq {
        ($left:expr, $right:expr, $msg:literal $(,)?) => {
            if $left != $right {
                error_list.push(format!("{}: {} <> {}", $msg, $left, $right));
            }
        };
        ($left:expr, $right:expr, $msg:literal, $($arg:tt)+) => {
            if $left != $right {
                error_list.push(format!("{}: {} <> {}", format!($msg, $($arg)+), $left, $right));
            }
        };
    }

    if logs_root1 != logs_root2 {
        let logs1 = exec_result1.as_ref().map(|r| r.logs()).unwrap_or_default();
        println!("ORIGINAL logs ({}):", logs1.len());
        for log in logs1 {
            println!(
                " - {}: {}",
                hex::encode(log.address),
                log.topics().first().map(hex::encode).unwrap_or_default()
            )
        }
        let logs2 = exec_result2.as_ref().map(|r| r.logs()).unwrap_or_default();
        println!("FLUENT logs ({}):", logs2.len());
        for log in logs2 {
            println!(
                " - {}: {}",
                hex::encode(log.address),
                log.topics().first().map(hex::encode).unwrap_or_default()
            )
        }
        error_eq!(logs_root1, logs_root2, "EVM <> FLUENT logs root mismatch");
    }

    let exec_result1_res = exec_result1.as_ref().unwrap();
    let exec_result2_res = exec_result2.as_ref().unwrap();
    error_eq!(
        exec_result1_res.is_success(),
        exec_result2_res.is_success(),
        "EVM <> FLUENT success status mismatch"
    );
    error_eq!(
        matches!(exec_result1_res, ExecutionResult::Revert { .. }),
        matches!(exec_result2_res, ExecutionResult::Revert { .. }),
        "EVM <> FLUENT revert status mismatch"
    );
    // Fluent returns deployed code through account metadata for successful
    // CREATEs. Compare that code below; CALL and REVERT return bytes directly.
    let is_create = matches!(
        exec_result1_res,
        ExecutionResult::Success {
            output: Output::Create(..),
            ..
        }
    );
    if !is_create && exec_result1_res.output() != exec_result2_res.output() {
        error_list.push(format!(
            "EVM <> FLUENT output mismatch: {:?} <> {:?}",
            exec_result1_res.output(),
            exec_result2_res.output()
        ));
    }
    error_eq!(
        exec_result1_res.tx_gas_used(),
        exec_result2_res.tx_gas_used(),
        "EVM <> FLUENT gas used mismatch"
    );

    // compare contracts
    // for (k, v) in evm.journaled_state.database.cache.contracts.iter() {
    //     let v2 = evm2
    //         .0
    //         .journaled_state
    //         .database
    //         .cache
    //         .contracts
    //         .get(k)
    //         .expect("missing fluent contract");
    //     // we compare only evm bytecode
    //     error_eq!(v.bytecode(), v2.bytecode(), "EVM bytecode mismatch");
    // }
    let mut account_keys = evm
        .journaled_state
        .database
        .cache
        .accounts
        .keys()
        .collect::<Vec<_>>();
    account_keys.sort();
    for address in account_keys {
        let v1 = evm
            .journaled_state
            .database
            .cache
            .accounts
            .get(address)
            .unwrap();
        if cfg!(feature = "debug-print") {
            println!("comparing account (0x{})...", hex::encode(address));
        }
        let v2 = evm2.0.journaled_state.database.cache.accounts.get(address);
        if let Some(a1) = v1.account.as_ref().map(|v| &v.info) {
            let a2 = v2
                .unwrap_or_else(|| panic!("missing FLUENT account {address}: native={v1:?}"))
                .account
                .as_ref()
                .map(|v| &v.info)
                .unwrap_or_else(|| {
                    panic!("missing FLUENT account {address}: native={v1:?}, fluent={v2:?}")
                });
            if cfg!(feature = "debug-print") {
                println!(" - status: {:?}", v1.status);
            }
            // error_eq!(
            //     format!("{:?}", v1.status),
            //     format!("{:?}", v2.unwrap().status),
            //     "EVM account status mismatch"
            // );
            if cfg!(feature = "debug-print") {
                println!(" - balance: {}", a1.balance);
            }
            error_eq!(
                a1.balance,
                a2.balance,
                "EVM <> FLUENT account ({}) balance mismatch",
                address,
            );
            if cfg!(feature = "debug-print") {
                println!(" - nonce: {}", a1.nonce);
            }
            error_eq!(a1.nonce, a2.nonce, "EVM <> FLUENT account nonce mismatch");
            if cfg!(feature = "debug-print") {
                println!(" - code_hash: {}", hex::encode(a1.code_hash));
            }
            let physical_precompile = GENESIS_CONTRACTS.with(|contracts| {
                contracts
                    .iter()
                    .any(|(genesis_address, _, _)| genesis_address == address)
            });
            if !physical_precompile {
                let native_code = a1
                    .code
                    .as_ref()
                    .or_else(|| {
                        evm.journaled_state
                            .database
                            .cache
                            .contracts
                            .get(&a1.code_hash)
                    })
                    .map(Bytecode::original_bytes)
                    .unwrap_or_default();
                let fluent_code = a2
                    .code
                    .as_ref()
                    .or_else(|| {
                        evm2.0
                            .journaled_state
                            .database
                            .cache
                            .contracts
                            .get(&a2.code_hash)
                    })
                    .map(|code| match code {
                        Bytecode::OwnableAccount(account)
                            if account.owner_address == PRECOMPILE_EVM_RUNTIME =>
                        {
                            fluentbase_evm::EthereumMetadata::read_from_bytes(&account.metadata)
                                .expect("invalid Fluent EVM metadata")
                                .code_copy()
                        }
                        other => other.original_bytes(),
                    })
                    .unwrap_or_default();
                error_eq!(
                    native_code,
                    fluent_code,
                    "EVM <> FLUENT account ({}) code mismatch",
                    address
                );
            }
            if cfg!(feature = "debug-print") {
                println!(" - storage:");
            }
            if let Some(s1) = v1.account.as_ref().map(|v| &v.storage) {
                let mut sorted_keys = s1.keys().collect::<Vec<_>>();
                sorted_keys.sort();
                for slot in sorted_keys {
                    let value1 = s1.get(slot).unwrap();
                    if cfg!(feature = "debug-print") {
                        println!(
                            " - + slot ({}) => ({})",
                            hex::encode(slot.to_be_bytes::<32>()),
                            hex::encode(value1.to_be_bytes::<32>())
                        );
                    }
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
                        .expect("missing FLUENT account (cache)")
                        .account
                        .as_ref()
                        .map(|v| &v.storage);
                    let value2 = value2
                        .expect("missing FLUENT account (storage)")
                        .get(slot)
                        .unwrap_or_else(|| {
                            error_list.push(format!(
                                "missing storage key {}",
                                hex::encode(slot.to_be_bytes::<32>())
                            ));
                            &U256::ZERO
                        });
                    error_eq!(
                        *value1,
                        *value2,
                        "EVM <> FLUENT storage value ({}) mismatch",
                        hex::encode(slot.to_be_bytes::<32>()),
                    );
                }
            }
        }
    }

    for (address, v1) in evm.journaled_state.database.cache.accounts.iter() {
        if cfg!(feature = "debug-print") {
            println!("comparing balances (0x{})...", hex::encode(address));
        }
        let v2 = evm2.0.journaled_state.database.cache.accounts.get(address);
        if let Some(a1) = v1.account.as_ref().map(|v| &v.info) {
            let a2 = v2
                .expect("missing FLUENT account")
                .account
                .as_ref()
                .map(|v| &v.info)
                .expect("missing FLUENT account");
            if cfg!(feature = "debug-print") {
                println!(" - balance1: {}", a1.balance);
                println!(" - balance2: {}", a2.balance);
            }
            let balance_diff = if a1.balance > a2.balance {
                a1.balance - a2.balance
            } else {
                a2.balance - a1.balance
            };
            if balance_diff != U256::from(0) {
                error_eq!(
                    a1.balance,
                    a2.balance,
                    "EVM <> FLUENT account balance mismatch"
                );
            }
        }
    }

    assert!(
        error_list.is_empty(),
        "Test {test_name}:\n----------------------\n{}\n----------------------\n",
        error_list.join("\n")
    );

    print_json_output(None);

    // if state_root1 != state_root2 {
    //     let kind = TestErrorKind::StateRootMismatch2 {
    //         expected: state_root1,
    //         got: state_root2,
    //     };
    //     print_json_output(Some(kind.to_string()));
    //     return Err(TestError {
    //         name: test_name.to_string(),
    //         kind,
    //     });
    // }

    Ok(())
}

fn format_evm_result(
    exec_result: &Result<
        ExecutionResult<HaltReason>,
        EVMError<EvmDatabaseError<Infallible>, InvalidTransaction>,
    >,
) -> String {
    match exec_result {
        Ok(r) => match r {
            ExecutionResult::Success { reason, .. } => format!("Success: {reason:?}"),
            ExecutionResult::Revert { .. } => "Revert".to_string(),
            ExecutionResult::Halt { reason, .. } => format!("Halt: {reason:?}"),
        },
        Err(e) => e.to_string(),
    }
}

fn build_json_output(
    test: &Test,
    test_name: &str,
    exec_result: &Result<
        ExecutionResult<HaltReason>,
        EVMError<EvmDatabaseError<Infallible>, InvalidTransaction>,
    >,
    validation: &TestValidationResult,
    spec: SpecId,
    error: Option<String>,
) -> serde_json::Value {
    json!({
        "stateRoot": validation.state_root,
        "logsRoot": validation.logs_root,
        "output": exec_result.as_ref().ok().and_then(|r| r.output().cloned()).unwrap_or_default(),
        "gasUsed": exec_result.as_ref().ok().map(|r| r.tx_gas_used()).unwrap_or_default(),
        "pass": error.is_none(),
        "errorMsg": error.unwrap_or_default(),
        "evmResult": format_evm_result(exec_result),
        "postLogsHash": validation.logs_root,
        "fork": spec,
        "test": test_name,
        "d": test.indexes.data,
        "g": test.indexes.gas,
        "v": test.indexes.value,
    })
}

fn validate_exception(
    test: &Test,
    exec_result: &Result<
        ExecutionResult<HaltReason>,
        EVMError<EvmDatabaseError<Infallible>, InvalidTransaction>,
    >,
) -> Result<bool, TestErrorKind> {
    match (&test.expect_exception, exec_result) {
        (None, Ok(_)) => Ok(false), // No exception expected, execution succeeded
        (Some(_), Err(_)) => Ok(true), // Exception expected and occurred
        _ => Err(TestErrorKind::UnexpectedException {
            expected_exception: test.expect_exception.clone(),
            got_exception: exec_result.as_ref().err().map(|e| e.to_string()),
        }),
    }
}

fn validate_output(
    expected_output: Option<&Bytes>,
    actual_result: &ExecutionResult<HaltReason>,
) -> Result<(), TestErrorKind> {
    if let Some((expected, actual)) = expected_output.zip(actual_result.output()) {
        if expected != actual {
            return Err(TestErrorKind::UnexpectedOutput {
                expected_output: Some(expected.clone()),
                got_output: actual_result.output().cloned(),
            });
        }
    }
    Ok(())
}

fn check_fluent_execution(
    test: &Test,
    expected_output: Option<&Bytes>,
    test_name: &str,
    exec_result: &Result<
        ExecutionResult<HaltReason>,
        EVMError<EvmDatabaseError<Infallible>, InvalidTransaction>,
    >,
    db: &mut State<EmptyDB>,
    spec: SpecId,
    print_json_outcome: bool,
) -> Result<(), TestErrorKind> {
    let validation = compute_test_roots(exec_result, db);

    let print_json = |error: Option<&TestErrorKind>| {
        if print_json_outcome {
            let json = build_json_output(
                test,
                test_name,
                exec_result,
                &validation,
                spec,
                error.map(|e| e.to_string()),
            );
            eprintln!("{json}");
        }
    };

    // Check if exception handling is correct
    let exception_expected = validate_exception(test, exec_result).inspect_err(|e| {
        print_json(Some(e));
    })?;

    // If exception was expected and occurred, we're done
    if exception_expected {
        print_json(None);
        return Ok(());
    }

    // Validate output if execution succeeded
    if let Ok(result) = exec_result {
        validate_output(expected_output, result).inspect_err(|e| {
            print_json(Some(e));
        })?;
    }

    // Validate logs root
    if validation.logs_root != test.logs {
        let error = TestErrorKind::LogsRootMismatch {
            got: validation.logs_root,
            expected: test.logs,
        };
        print_json(Some(&error));
        return Err(error);
    }

    match exec_result {
        Ok(result) => println!(
            "Execution result: gas_used={} success={} logs={} output_len={}",
            result.tx_gas_used(),
            result.is_success(),
            result.logs().len(),
            result.output().map(|o| o.len()).unwrap_or(0)
        ),
        Err(err) => println!("Execution result: error={err:?}"),
    }
    for (k, account_should_be) in &test.state {
        println!("Checking account: {k}");
        let actual_account = db
            .load_cache_account(*k)
            .map_err(|_| TestErrorKind::MissingAccount { address: *k })?;
        let actual_account = actual_account.account.clone().unwrap().info;
        assert_eq!(
            actual_account.balance,
            account_should_be.balance,
            "balance mismatch for {k}: diff (actual - expected) = {}",
            actual_account.balance.abs_diff(account_should_be.balance)
        );
        if let Some(code) = actual_account.code.as_ref() {
            assert_eq!(code.original_bytes(), account_should_be.code);
        } else {
            assert!(actual_account.code.is_none());
        }
        assert_eq!(actual_account.nonce, account_should_be.nonce);
        for (sk, sv) in &account_should_be.storage {
            let actual_value = db.storage(*k, *sk).unwrap();
            assert_eq!(actual_value, *sv);
        }
    }

    // Validate state root
    if test.hash != B256::ZERO && validation.state_root != test.hash {
        let error = TestErrorKind::StateRootMismatch {
            got: validation.state_root,
            expected: test.hash,
        };
        print_json(Some(&error));
        return Err(error);
    }

    print_json(None);
    Ok(())
}

pub fn execute_evm_test_suite(
    path: &Path,
    elapsed: &Arc<Mutex<Duration>>,
    trace: bool,
    print_json_outcome: bool,
) -> Result<ExecutionStats, TestError> {
    if cfg!(feature = "debug-print") {
        println!("Running test: {:?}", path);
    }

    let s = std::fs::read_to_string(path).unwrap();
    let mut raw_suite: Value = serde_json::from_str(&s).map_err(|e| TestError {
        name: path.to_string_lossy().into_owned(),
        kind: e.into(),
    })?;
    // Modern signature-validation fixtures provide a sender and txbytes without
    // a private key. The pinned TestUnit schema still requires secretKey; its
    // value is unused when sender is present, and txbytes are validated below.
    if let Some(units) = raw_suite.as_object_mut() {
        for unit in units.values_mut() {
            if let Some(transaction) = unit.get_mut("transaction").and_then(Value::as_object_mut) {
                if transaction.contains_key("sender") && !transaction.contains_key("secretKey") {
                    transaction.insert("secretKey".to_owned(), json!(B256::ZERO));
                }
            }
        }
    }
    let suite: TestSuite = serde_json::from_value(raw_suite.clone()).map_err(|e| TestError {
        name: path.to_string_lossy().into_owned(),
        kind: e.into(),
    })?;
    let mut stats = eligible_post_cases(&suite, path)?;

    let selected_test_cases = Vec::new();
    for (name, unit) in suite.0 {
        if !selected_test_cases.is_empty() && !selected_test_cases.contains(&name.as_str()) {
            continue;
        }
        if cfg!(feature = "debug-print") {
            println!("test case: {}", &name);
        }
        // Create a database and insert a cache
        let evm_cache_state = evm_cache_state(&unit);
        let mut fluent_cache_state = fluent_cache_state(&unit);

        let start = Instant::now();
        if cfg!(feature = "debug-print") {
            println!("\nloading genesis accounts:");
        }
        let genesis_contracts = GENESIS_CONTRACTS.with(Clone::clone);
        for (address, code_hash, bytecode) in genesis_contracts.iter() {
            // Match historical execute_test_suite merge semantics:
            // - keep prestate balance/nonce/storage for existing accounts
            // - but still enforce genesis runtime bytecode at known genesis addresses
            if let Some(existing) = fluent_cache_state.accounts.get_mut(address) {
                if let Some(account) = existing.account.as_mut() {
                    account.info.code_hash = *code_hash;
                    account.info.code = Some(bytecode.clone());
                }
                continue;
            }
            let acc_info = AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: *code_hash,
                account_id: None,
                code: Some(bytecode.clone()),
            };
            fluent_cache_state.insert_account(*address, acc_info);
        }
        if cfg!(feature = "debug-print") {
            println!("loaded genesis accounts in: {:?}", start.elapsed());
        }

        let (mut cfg_env, _, mut tx_env) = prepare_env(&unit, &name)?;
        // The pinned upstream TransactionParts omits chainId. Preserve it from
        // the fixture so invalid-chain transactions reach both engines intact.
        if let Some(chain_id) = raw_suite[&name]["transaction"].get("chainId") {
            let chain_id: U256 =
                serde_json::from_value(chain_id.clone()).map_err(|e| TestError {
                    name: name.clone(),
                    kind: e.into(),
                })?;
            tx_env.chain_id = Some(chain_id.to());
        }

        for (spec_name, tests) in &unit.post {
            // The delegated EVM and built-in precompiles execute Osaka semantics,
            // independently of the host chain's hardfork schedule.
            if *spec_name != SpecName::Osaka {
                continue;
            }

            let spec_id = spec_name.to_spec_id();
            cfg_env.set_spec_and_mainnet_gas_params(spec_id);
            cfg_env.set_max_blobs_per_tx(if spec_id.is_enabled_in(SpecId::OSAKA) {
                6
            } else {
                9
            });
            let block_env = unit.block_env(&mut cfg_env);

            for (index, test) in tests.iter().enumerate() {
                if let Some(reason) = excluded_case_reason(path, &name, test) {
                    println!("excluded {name}: {reason}");
                    stats.skipped += 1;
                    continue;
                }
                if let Some(bytes) = &test.txbytes {
                    match signed_transaction_sender_and_chain_id(bytes) {
                        Ok((sender, chain_id)) => {
                            if sender != tx_env.caller {
                                return Err(TestError {
                                    name: name.clone(),
                                    kind: TestErrorKind::UnexpectedException {
                                        expected_exception: None,
                                        got_exception: Some(
                                            "signed transaction sender differs from fixture sender"
                                                .to_owned(),
                                        ),
                                    },
                                });
                            }
                            tx_env.chain_id = chain_id;
                        }
                        Err(error) => {
                            // Invalid signatures and malformed envelopes are admission
                            // tests, not VM executions. Report their coverage separately.
                            if test.expect_exception.is_none() {
                                return Err(TestError {
                                    name: name.clone(),
                                    kind: TestErrorKind::UnexpectedException {
                                        expected_exception: None,
                                        got_exception: Some(error),
                                    },
                                });
                            }
                            let state_root = state_merkle_trie_root(evm_cache_state.trie_account());
                            if state_root != test.hash {
                                return Err(TestError {
                                    name: name.clone(),
                                    kind: TestErrorKind::StateRootMismatch {
                                        got: state_root,
                                        expected: test.hash,
                                    },
                                });
                            }
                            let logs_root = log_rlp_hash(&[]);
                            if logs_root != test.logs {
                                return Err(TestError {
                                    name: name.clone(),
                                    kind: TestErrorKind::LogsRootMismatch {
                                        got: logs_root,
                                        expected: test.logs,
                                    },
                                });
                            }
                            stats.rejected_before_execution += 1;
                            continue;
                        }
                    }
                }
                if cfg!(feature = "debug-print") {
                    println!(
                        "\n\n\n\n\nRunning test with txdata: ({}) {}",
                        index,
                        hex::encode(test.txbytes.clone().unwrap_or_default().as_ref())
                    );
                }
                fill_tx_env(&mut tx_env, &unit.transaction, test);

                let evm_cache = evm_cache_state.clone();
                // evm_cache.set_state_clear_flag(spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON));
                let fluent_cache = fluent_cache_state.clone();
                // fluent_cache.set_state_clear_flag(spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON));

                let evm_state: State<InMemoryDB> = StateBuilder::default()
                    .with_cached_prestate(evm_cache)
                    .with_bundle_update()
                    .build();
                let fluent_state: State<InMemoryDB> = StateBuilder::default()
                    .with_cached_prestate(fluent_cache)
                    .with_bundle_update()
                    .build();
                let output = if trace {
                    let timer = Instant::now();
                    if cfg!(feature = "debug-print") {
                        print!("\n\nrunning original EVM tests... ");
                    }
                    let mut evm = MainnetContext::new(evm_state, spec_id)
                        .with_cfg(cfg_env.clone())
                        .with_block(block_env.clone())
                        .build_mainnet_with_inspector(TraceInspector::new());
                    evm.cfg.legacy_bytecode_enabled = true;
                    let start = Instant::now();
                    let result_native = evm.inspect_tx_commit(tx_env.clone());
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                    }
                    let start = Instant::now();
                    if cfg!(feature = "debug-print") {
                        print!("\n\nrunning RWASM tests... ");
                    }
                    let mut evm2 = RwasmContext::new(fluent_state, spec_id)
                        .with_cfg(cfg_env.clone())
                        .with_block(block_env.clone())
                        .build_rwasm_with_inspector(TraceInspector::new());
                    evm2.0.cfg.legacy_bytecode_enabled = false;
                    let result_fluent = evm2.inspect_tx_commit(tx_env.clone());
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                    }
                    *elapsed.lock().unwrap() += timer.elapsed();
                    // dump state and traces if the test failed
                    let start = Instant::now();
                    if cfg!(feature = "debug-print") {
                        print!("\n\ncomparing EVM<>RWASM state... ");
                    }
                    let output = check_evm_execution(
                        test,
                        unit.out.as_ref(),
                        &name,
                        &result_native,
                        &result_fluent,
                        &mut evm,
                        &mut evm2,
                        print_json_outcome,
                    );
                    // check_evm_trace(evm.inspector(), evm2.inspector())?;
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                    }
                    output
                } else {
                    let timer = Instant::now();
                    if cfg!(feature = "debug-print") {
                        print!("\n\nrunning original EVM tests... ");
                    }
                    let mut evm = MainnetContext::new(evm_state, spec_id)
                        .with_cfg(cfg_env.clone())
                        .with_block(block_env.clone())
                        .build_mainnet();
                    evm.cfg.legacy_bytecode_enabled = true;
                    let start = Instant::now();
                    let result_native = evm.transact_commit(tx_env.clone());
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                        print!("\n\nrunning RWASM tests... ");
                    }
                    let mut evm2 = RwasmContext::new(fluent_state, spec_id)
                        .with_cfg(cfg_env.clone())
                        .with_block(block_env.clone())
                        .build_rwasm();
                    evm2.0.cfg.legacy_bytecode_enabled = false;
                    let start = Instant::now();
                    let result_fluent = evm2.transact_commit(tx_env.clone());
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                    }
                    *elapsed.lock().unwrap() += timer.elapsed();
                    // dump state and traces if the test failed
                    let start = Instant::now();
                    if cfg!(feature = "debug-print") {
                        print!("\n\ncomparing EVM<>RWASM state... ");
                    }
                    let output = check_evm_execution(
                        test,
                        unit.out.as_ref(),
                        &name,
                        &result_native,
                        &result_fluent,
                        &mut evm,
                        &mut evm2,
                        print_json_outcome,
                    );
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                    }
                    output
                };

                stats.executed += 1;
                let Err(e) = output else {
                    continue;
                };

                // if we are already in trace mode, return error
                static FAILED: AtomicBool = AtomicBool::new(false);
                if trace || FAILED.swap(true, Ordering::SeqCst) {
                    return Err(e);
                }

                return Err(e);
            }
        }
    }
    if stats.executed == 0 && stats.rejected_before_execution == 0 {
        return Err(TestError {
            name: path.display().to_string(),
            kind: TestErrorKind::NoTransactionsExecuted {
                skipped: stats.skipped,
            },
        });
    }
    println!(
        "{}: executed {} transactions, validated {} rejected envelopes, skipped {} post cases",
        path.display(),
        stats.executed,
        stats.rejected_before_execution,
        stats.skipped
    );
    Ok(stats)
}

pub fn resolve_externalized_bytecodes(v: &mut Value, base_dir: &Path) {
    match v {
        Value::Array(arr) => {
            for item in arr {
                resolve_externalized_bytecodes(item, base_dir);
            }
        }
        Value::Object(map) => {
            for (_, value) in map.iter_mut() {
                resolve_externalized_bytecodes(value, base_dir);
            }
        }
        Value::String(s) => {
            if let Some(file) = s.strip_prefix("file://fixtures/reusable-bytecode/") {
                let path: PathBuf = base_dir.join("reusable-bytecode").join(file);
                let bytes = fs::read(&path).unwrap();
                *s = format!("0x{}", hex::encode(bytes));
            }
        }
        _ => {}
    }
}

/// The precompile provider the node installs for block execution.
///
/// `crates/node/src/evm.rs` wraps `RwasmPrecompiles::precompiles()` (an empty set: Fluent
/// precompiles are genesis rWASM contracts) in reth's `PrecompilesMap`, whose warm-address list is
/// that same empty set. On chain no precompile address is therefore pre-warmed at transaction
/// start, and the first call to one pays the cold account-access cost. The library default,
/// `RwasmPrecompiles::warm_addresses`, pre-warms the canonical EIP-2929 list instead, which
/// under-charges such a call by 2500 gas against canonical receipts. Fixtures replay real
/// transactions, so they must use the node's semantics.
fn node_precompiles(spec_id: SpecId) -> EthPrecompiles {
    EthPrecompiles {
        precompiles: RwasmPrecompiles::new_with_spec(spec_id).precompiles(),
        spec: spec_id,
    }
}

pub fn execute_fluent_test_suite(
    path: &Path,
    elapsed: &Arc<Mutex<Duration>>,
    trace: bool,
    print_json_outcome: bool,
) -> Result<(), TestError> {
    if skip_test(path) {
        return Ok(());
    }

    if cfg!(feature = "debug-print") {
        println!("Running test: {:?}", path);
    }

    let mut fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    resolve_externalized_bytecodes(&mut fixture, path.parent().unwrap());

    let suite: TestSuite = serde_json::from_value(fixture).map_err(|e| TestError {
        name: path.to_string_lossy().into_owned(),
        kind: e.into(),
    })?;

    for (name, unit) in suite.0 {
        if cfg!(feature = "debug-print") {
            println!("test case: {}", &name);
        }

        let cache_state = evm_cache_state(&unit);
        let (mut cfg_env, block_env, mut tx_env) = prepare_env(&unit, &name)?;

        // TODO(dmitry123): Once revm testing unit has config use it instead of path check
        if path.to_str().unwrap().contains("testnet") {
            cfg_env.chain_id = 20994;
        } else if path.to_str().unwrap().contains("devnet") {
            cfg_env.chain_id = 20993;
        } else {
            cfg_env.chain_id = 25363;
        }

        for (spec_name, tests) in unit.post {
            // Fluent is post-PRAGUE only
            if spec_name.lt(&SpecName::Prague) {
                continue;
            }

            let spec_id = spec_name.to_spec_id();
            cfg_env.spec = spec_id;

            for (index, test) in tests.into_iter().enumerate() {
                if cfg!(feature = "debug-print") {
                    println!(
                        "\n\n\n\n\nRunning test with txdata: ({}) {}",
                        index,
                        hex::encode(test.txbytes.clone().unwrap_or_default().as_ref())
                    );
                }
                fill_tx_env(&mut tx_env, &unit.transaction, &test);
                tx_env.chain_id = Some(cfg_env.chain_id);

                let cache = cache_state.clone();
                // cache.set_state_clear_flag(spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON));

                let state: State<EmptyDB> = StateBuilder::default()
                    .with_cached_prestate(cache)
                    .with_bundle_update()
                    .build();
                let output = if trace {
                    let start = Instant::now();
                    let mut evm = RwasmContext::new(state, spec_id)
                        .with_cfg(cfg_env.clone())
                        .with_block(block_env.clone())
                        .build_rwasm_with_inspector(TraceInspector::new())
                        .with_precompiles(node_precompiles(spec_id));
                    evm.0.cfg.legacy_bytecode_enabled = false;
                    let result_fluent = evm.inspect_tx_commit(tx_env.clone());
                    *elapsed.lock().unwrap() += start.elapsed();
                    let output = check_fluent_execution(
                        &test,
                        unit.out.as_ref(),
                        &name,
                        &result_fluent,
                        evm.0.db_mut(),
                        spec_id,
                        print_json_outcome,
                    );
                    output
                } else {
                    let mut evm = RwasmContext::new(state, spec_id)
                        .with_cfg(cfg_env.clone())
                        .with_block(block_env.clone())
                        .build_rwasm()
                        .with_precompiles(node_precompiles(spec_id));
                    evm.0.cfg.legacy_bytecode_enabled = false;
                    let timer = Instant::now();
                    let result = evm.transact_commit(tx_env.clone());
                    *elapsed.lock().unwrap() += timer.elapsed();
                    let start = Instant::now();
                    let output = check_fluent_execution(
                        &test,
                        unit.out.as_ref(),
                        &name,
                        &result,
                        evm.0.db_mut(),
                        spec_id,
                        print_json_outcome,
                    );
                    if cfg!(feature = "debug-print") {
                        println!("{:?}", start.elapsed());
                    }
                    output
                };

                let Err(e) = output else {
                    continue;
                };

                // if we are already in trace mode, return error
                static FAILED: AtomicBool = AtomicBool::new(false);
                if trace || FAILED.swap(true, Ordering::SeqCst) {
                    return Err(TestError { name, kind: e });
                }

                return Err(TestError { name, kind: e });
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
    keep_going: bool,
) -> Result<(), TestError> {
    if test_files.is_empty() {
        return Err(TestError {
            name: "fixture selection".to_string(),
            kind: TestErrorKind::NoTransactionsExecuted { skipped: 0 },
        });
    }
    // trace implies print_outcome
    if trace {
        print_outcome = true;
    }
    // print_outcome or trace implies single_thread
    if print_outcome {
        single_thread = true;
    }
    let n_files = test_files.len();

    let n_errors = Arc::new(AtomicUsize::new(0));
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
        let n_errors = n_errors.clone();
        let console_bar = console_bar.clone();
        let elapsed = elapsed.clone();

        let thread = std::thread::Builder::new().name(format!("runner-{i}"));

        let f = move || loop {
            if !keep_going && n_errors.load(Ordering::SeqCst) > 0 {
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

            let result = execute_evm_test_suite(&test_path, &elapsed, trace, print_outcome);

            // Increment after the test is done.
            console_bar.inc(1);

            if let Err(err) = result {
                n_errors.fetch_add(1, Ordering::SeqCst);
                if !keep_going {
                    return Err(err);
                }
            }
        };
        handles.push(thread.spawn(f).unwrap());
    }

    // join all threads before returning an error
    let mut thread_errors = Vec::new();
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => thread_errors.push(e),
            Err(_) => thread_errors.push(TestError {
                name: format!("thread {i} panicked"),
                kind: TestErrorKind::Panic,
            }),
        }
    }
    console_bar.finish();

    println!(
        "Finished execution. Total CPU time: {:.6}s",
        elapsed.lock().unwrap().as_secs_f64()
    );

    let n_errors = n_errors.load(Ordering::SeqCst);
    let n_thread_errors = thread_errors.len();
    if n_errors == 0 && n_thread_errors == 0 {
        println!("All tests passed!");
        Ok(())
    } else {
        println!("Encountered {n_errors} errors out of {n_files} total tests");

        if n_thread_errors == 0 {
            std::process::exit(1);
        }

        if n_thread_errors > 1 {
            println!("{n_thread_errors} threads returned an error, out of {num_threads} total:");
            for error in &thread_errors {
                println!("{error}");
            }
        }
        Err(thread_errors.swap_remove(0))
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn execute_selection(suite: Value) -> Result<ExecutionStats, TestError> {
        static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "fluentbase-state-selection-{}-{}.json",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, serde_json::to_vec(&suite).unwrap()).unwrap();
        let elapsed = Arc::new(Mutex::new(Duration::ZERO));
        let result = execute_evm_test_suite(&path, &elapsed, false, false);
        fs::remove_file(path).unwrap();
        result
    }

    fn fixture_for_fork(fork: &str) -> Value {
        let mut suite = json!({
            "selection_regression": {
                "env": {
                    "currentCoinbase": "0x0000000000000000000000000000000000000000",
                    "currentGasLimit": "0x100000", "currentNumber": "0x01",
                    "currentTimestamp": "0x01"
                },
                "pre": {},
                "transaction": {
                    "data": ["0x"], "gasLimit": ["0x5208"], "gasPrice": "0x01",
                    "nonce": "0x00", "value": ["0x00"], "secretKey": B256::ZERO
                },
                "post": {}
            }
        });
        suite["selection_regression"]["post"][fork] = json!([{
            "hash": B256::ZERO, "logs": B256::ZERO,
            "indexes": {"data": 0, "gas": 0, "value": 0}
        }]);
        suite
    }

    #[test]
    fn empty_suite_fails_instead_of_passing() {
        assert!(matches!(
            execute_selection(json!({})).unwrap_err().kind,
            TestErrorKind::NoTransactionsExecuted { skipped: 0 }
        ));
    }

    #[test]
    fn pre_prague_only_suite_fails_instead_of_passing() {
        assert!(matches!(
            execute_selection(fixture_for_fork("Cancun"))
                .unwrap_err()
                .kind,
            TestErrorKind::NoTransactionsExecuted { skipped: 1 }
        ));
    }

    #[test]
    fn prague_only_suite_does_not_compare_different_runtime_rules() {
        assert!(matches!(
            execute_selection(fixture_for_fork("Prague"))
                .unwrap_err()
                .kind,
            TestErrorKind::NoTransactionsExecuted { skipped: 1 }
        ));
    }

    #[test]
    fn unknown_fork_fails_before_execution() {
        assert!(matches!(
            execute_selection(fixture_for_fork("UnrecognizedFork"))
                .unwrap_err()
                .kind,
            TestErrorKind::UnsupportedFork(_)
        ));
    }

    #[test]
    fn empty_file_selection_fails_instead_of_passing() {
        assert!(matches!(
            run(Vec::new(), true, false, false, false).unwrap_err().kind,
            TestErrorKind::NoTransactionsExecuted { skipped: 0 }
        ));
    }
}
