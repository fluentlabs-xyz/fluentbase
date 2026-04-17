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
    }
}
