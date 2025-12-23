use core::str::from_utf8;
use fluentbase_contracts::FLUENTBASE_EXAMPLES_MEMORY_OOM;
use fluentbase_sdk::{Address, Bytes};
use fluentbase_testing::EvmTestingContext;
use revm::context::result::{ExecutionResult, HaltReason, OutOfGasError};
use rwasm::{CompilationConfig, RwasmModule};

#[test]
fn test_oom_has_proper_exit_code() {
    let mut ctx = EvmTestingContext::default();
    let contract_address = Address::with_last_byte(77);
    ctx.add_wasm_contract(
        contract_address,
        FLUENTBASE_EXAMPLES_MEMORY_OOM.wasm_bytecode,
    );
    const CALLER: Address = Address::with_last_byte(81);
    // call greeting WASM contract
    let result = ctx.call_evm_tx(CALLER, contract_address, Bytes::default(), None, None);
    assert_eq!(
        result,
        ExecutionResult::Halt {
            reason: HaltReason::OutOfGas(OutOfGasError::MemoryLimit),
            gas_used: 3_000_000
        }
    );
}
