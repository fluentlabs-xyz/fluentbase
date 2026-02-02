use crate::EvmTestingContextWithGenesis;
use fluentbase_contracts::FLUENTBASE_EXAMPLES_MEMORY_OOM;
use fluentbase_sdk::{Address, Bytes, PRECOMPILE_OAUTH2_VERIFIER};
use fluentbase_testing::EvmTestingContext;
use revm::context::result::{ExecutionResult, HaltReason, OutOfGasError};

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
    assert_eq!(
        result,
        ExecutionResult::Halt {
            reason: HaltReason::UnreachableCodeReached,
            gas_used: 3_000_000
        }
    );
}
