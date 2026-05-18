use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{Address, Bytes, PRECOMPILE_OAUTH2_VERIFIER};
use fluentbase_testing::EvmTestingContext;
use revm::context::result::{ExecutionResult, HaltReason};

#[test]
fn test_oauth2_should_not_panic() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const CALLER: Address = Address::with_last_byte(81);
    let result = ctx.call_evm_tx(
        CALLER,
        PRECOMPILE_OAUTH2_VERIFIER,
        Bytes::default(),
        None,
        None,
    );
    match result {
        ExecutionResult::Halt { reason, .. } => {
            assert_eq!(reason, HaltReason::UnreachableCodeReached);
        }
        _ => panic!("Unexpected execution result: {:?}", result),
    }
}
