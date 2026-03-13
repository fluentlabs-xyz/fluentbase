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
    }
}
