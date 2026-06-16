macro_rules! define_tests {
    (
        $( fn $test_name:ident($test_path:literal); )*
    ) => {
        $(
            #[test]
            fn $test_name() {
                $crate::utils::run_fluent_e2e_test($test_path)
            }
        )*
    };
}

mod fluent_testnet {
    define_tests! {
        fn testnet_20986140_legacy_ust20_params("fixtures/testnet_20986140_legacy_ust20_params.json");
        fn testnet_20987069_rwasm_gas_mismatch("fixtures/testnet_20987069_rwasm_gas_mismatch.json");
        fn testnet_22882338_hello_world("fixtures/testnet_22882338_hello_world.json");
        fn testnet_24293144_bridge_malformed_params_1("fixtures/testnet_24293144_bridge_malformed_params_1.json");
        fn testnet_24293146_bridge_malformed_params_2("fixtures/testnet_24293146_bridge_malformed_params_2.json");
        fn testnet_24293147_bridge_malformed_params_3("fixtures/testnet_24293147_bridge_malformed_params_3.json");
        fn testnet_24358755_bridge_failed_receive("fixtures/testnet_24358755_bridge_failed_receive.json");
        fn testnet_24359230_bridge_ether_received("fixtures/testnet_24359230_bridge_ether_received.json");
        fn testnet_24359651_bridge_ust20_received("fixtures/testnet_24359651_bridge_ust20_received.json");
        fn testnet_24363142_bridge_malformed_params_4("fixtures/testnet_24363142_bridge_malformed_params_4.json");
        fn testnet_24456528_bridge_double_mint("fixtures/testnet_24456528_bridge_double_mint.json");
        fn testnet_24457093_bridge_double_mint("fixtures/testnet_24457093_bridge_double_mint.json");
        fn testnet_eth_call_b7ab4db5_malformed_builtin_params("fixtures/testnet_eth_call_b7ab4db5_malformed_builtin_params.json");
    }
}

mod fluent_mainnet {
    define_tests! {
        fn mainnet_2697535_bridge_token_deployment("fixtures/mainnet_2697535_bridge_token_deployment.json");
        fn mainnet_2698630_bridge_token_deployment("fixtures/mainnet_2698630_bridge_token_deployment.json");
    }
}

#[test]
fn eth_call_b7ab4db5_halts_with_malformed_builtin_params() {
    use crate::{
        runner::resolve_externalized_bytecodes,
        state::{evm_cache_state, fill_tx_env, prepare_env},
    };
    use fluentbase_revm::{RwasmBuilder, RwasmContext};
    use revm::{
        context::result::ExecutionResult,
        database::{EmptyDB, StateBuilder},
        interpreter::InstructionResult,
        ExecuteCommitEvm,
    };
    use revm_statetest_types::{SpecName, TestSuite};
    use std::{fs, path::Path};

    let path = Path::new("./fixtures/testnet_eth_call_b7ab4db5_malformed_builtin_params.json");
    let mut fixture: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap())
        .expect("fixture should deserialize as json");
    resolve_externalized_bytecodes(&mut fixture, path.parent().unwrap());
    let suite: TestSuite = serde_json::from_value(fixture).expect("fixture should parse");
    let (name, unit) = suite
        .0
        .into_iter()
        .next()
        .expect("fixture should contain one test");

    let cache_state = evm_cache_state(&unit);
    let (mut cfg_env, block_env, mut tx_env) = prepare_env(&unit, &name).unwrap();
    cfg_env.chain_id = 20994;

    let tests = unit
        .post
        .into_iter()
        .find_map(|(spec_name, tests)| (spec_name == SpecName::Prague).then_some(tests))
        .expect("fixture should define Prague expectations");
    let test = tests
        .into_iter()
        .next()
        .expect("fixture should contain one post");

    cfg_env.spec = SpecName::Prague.to_spec_id();
    fill_tx_env(&mut tx_env, &unit.transaction, &test);
    tx_env.chain_id = Some(cfg_env.chain_id);

    let mut cache = cache_state.clone();
    cache.set_state_clear_flag(
        cfg_env
            .spec
            .is_enabled_in(revm::primitives::hardfork::SpecId::SPURIOUS_DRAGON),
    );
    let state: revm::database::State<EmptyDB> = StateBuilder::default()
        .with_cached_prestate(cache)
        .with_bundle_update()
        .build();
    let mut evm = RwasmContext::new(state, cfg_env.spec)
        .with_cfg(cfg_env)
        .with_block(block_env)
        .build_rwasm();
    evm.0.cfg.legacy_bytecode_enabled = false;

    let result = evm.transact_commit(tx_env);
    let Ok(ExecutionResult::Halt { reason, .. }) = result else {
        panic!("expected MalformedBuiltinParams halt, got {result:?}");
    };
    let instruction_result: InstructionResult = reason.into();
    assert_eq!(
        instruction_result,
        InstructionResult::MalformedBuiltinParams,
        "0xb7 is PUSH24 and the payload only supplies three immediate bytes"
    );
}
