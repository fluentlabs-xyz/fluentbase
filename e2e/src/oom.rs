use crate::EvmTestingContextWithGenesis;
use fluentbase_contracts::FLUENTBASE_EXAMPLES_MEMORY_OOM;
use fluentbase_sdk::{Address, Bytes, ExitCode::MalformedBuiltinParams};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use revm::context::result::{ExecutionResult, HaltReason};

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
            reason: HaltReason::MemoryOutOfBounds,
            gas_used: 3_000_000
        }
    );
}

#[test]
fn test_negative_write_output_params_cant_cause_oom() {
    let wasm_module: Bytes = wat::parse_str(
        r#"
(module
  (import "fluentbase_v1preview" "_write" (func $_write (param i32 i32)))
  (memory (export "memory") 0)
  (func (export "main")
    unreachable
  )
  (func (export "deploy")
    i32.const 0
    i32.const 134217728
    call $_write
  )
)
    "#,
    )
    .unwrap()
    .into();
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let result = TxBuilder::create(&mut ctx, Address::repeat_byte(0x01), wasm_module).exec();
    assert_eq!(
        result,
        ExecutionResult::Halt {
            reason: HaltReason::MemoryOutOfBounds,
            gas_used: 100_000_000
        }
    )
}
