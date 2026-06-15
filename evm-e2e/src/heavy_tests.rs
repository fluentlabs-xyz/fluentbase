const RUN_HEAVY_EVM_REPRO_ENV: &str = "FLUENTBASE_RUN_HEAVY_EVM_REPRO";

#[test]
fn synthetic_mstore8_fixture_deserializes() {
    let fixture = include_str!("../fixtures/heavy/evm_memory_2gib_mstore8.json");
    let _: revm_statetest_types::TestSuite = serde_json::from_str(fixture).unwrap();
}

fn run_heavy_evm_e2e_test(test_path: &'static str) {
    if std::env::var(RUN_HEAVY_EVM_REPRO_ENV).as_deref() != Ok("1") {
        panic!(
            "set {RUN_HEAVY_EVM_REPRO_ENV}=1 to run heavy EVM repro test {test_path}; \
             these tests can allocate large memory and are intentionally opt-in"
        );
    }

    crate::utils::run_evm_e2e_test(test_path);
}

macro_rules! define_heavy_tests {
    (
        $( fn $test_name:ident($test_path:literal); )*
    ) => {
        $(
            #[test]
            #[ignore = "heavy EVM repro; set FLUENTBASE_RUN_HEAVY_EVM_REPRO=1"]
            fn $test_name() {
                run_heavy_evm_e2e_test($test_path)
            }
        )*
    };
}

mod synthetic_memory {
    use super::*;

    define_heavy_tests! {
        fn mstore8_2gib("fixtures/heavy/evm_memory_2gib_mstore8.json");
    }
}

mod upstream_st_quadratic_complexity {
    use super::*;

    define_heavy_tests! {
        fn call1_mb1024_calldepth("tests/GeneralStateTests/stQuadraticComplexityTest/Call1MB1024Calldepth.json");
        fn call20_kbytes_contract50_1("tests/GeneralStateTests/stQuadraticComplexityTest/Call20KbytesContract50_1.json");
        fn call20_kbytes_contract50_2("tests/GeneralStateTests/stQuadraticComplexityTest/Call20KbytesContract50_2.json");
        fn call20_kbytes_contract50_3("tests/GeneralStateTests/stQuadraticComplexityTest/Call20KbytesContract50_3.json");
        fn call50000("tests/GeneralStateTests/stQuadraticComplexityTest/Call50000.json");
        fn call50000_ecrec("tests/GeneralStateTests/stQuadraticComplexityTest/Call50000_ecrec.json");
        fn call50000_identity("tests/GeneralStateTests/stQuadraticComplexityTest/Call50000_identity.json");
        fn call50000_identity2("tests/GeneralStateTests/stQuadraticComplexityTest/Call50000_identity2.json");
        fn call50000_rip160("tests/GeneralStateTests/stQuadraticComplexityTest/Call50000_rip160.json");
        fn call50000_sha256("tests/GeneralStateTests/stQuadraticComplexityTest/Call50000_sha256.json");
        fn callcode50000("tests/GeneralStateTests/stQuadraticComplexityTest/Callcode50000.json");
        fn create1000("tests/GeneralStateTests/stQuadraticComplexityTest/Create1000.json");
        fn create1000_byzantium("tests/GeneralStateTests/stQuadraticComplexityTest/Create1000Byzantium.json");
        fn create1000_shnghai("tests/GeneralStateTests/stQuadraticComplexityTest/Create1000Shnghai.json");
        fn quadratic_complexity_solidity_call_data("tests/GeneralStateTests/stQuadraticComplexityTest/QuadraticComplexitySolidity_CallDataCopy.json");
        fn return50000("tests/GeneralStateTests/stQuadraticComplexityTest/Return50000.json");
        fn return50000_2("tests/GeneralStateTests/stQuadraticComplexityTest/Return50000_2.json");
    }
}
