use super::initcode_size_limit_exceeded;
use fluentbase_sdk::EVM_MAX_INITCODE_SIZE;

#[test]
fn initcode_size_limit_matches_contract_size_mode() {
    assert!(!initcode_size_limit_exceeded(EVM_MAX_INITCODE_SIZE));

    #[cfg(not(feature = "permissive-contract-size"))]
    assert!(initcode_size_limit_exceeded(EVM_MAX_INITCODE_SIZE + 1));

    #[cfg(feature = "permissive-contract-size")]
    assert!(!initcode_size_limit_exceeded(EVM_MAX_INITCODE_SIZE + 1));
}
