use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{calc_create_address, Address, Bytes};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use wat::parse_str;

const EXEC_BALANCE_DOS_WAT: &str = r#"
    (module
        (import "fluentbase_v1preview" "_read" (func $_read (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_exec" (func $_exec (param i32 i32 i32 i32 i32) (result i32)))
        (import "fluentbase_v1preview" "_exit" (func $_exit (param i32)))
        (memory 1)
        (data (i32.const 64)
            "\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\0b")
        (func $main (local $len i32)
            ;; Read the caller-supplied length into memory[0..4].
            ;;
            ;; Note that the first 1024 bytes are encoded context from the system, so we need to load from byte 1024
            ;; https://github.com/fluentlabs-xyz/fluentbase/blob/e88ea5712c2eb568a6cd9c8946db48064de41ab0/crates/revm/src/executor.rs#L154-L158
            i32.const 0    ;; target ptr
            i32.const 1024 ;; offset
            i32.const 4    ;; length
            call $_read

            i32.const 0
            i32.load
            local.set $len

            ;; Provision a large memory buffer up-front so both test cases have identical growth costs.
            i32.const 512
            memory.grow
            drop

            ;; Invoke the BALANCE syscall via _exec with an attacker-controlled input length.
            i32.const 64      ;; pointer to SYSCALL_ID_BALANCE
            i32.const 128     ;; input pointer
            local.get $len    ;; attacker-chosen length
            i32.const 0       ;; no explicit fuel limit
            i32.const 0       ;; STATE_MAIN
            call $_exec
            drop

            ;; Exit without bubbling up the nested error.
            i32.const 0
            call $_exit
        )
        (export "main" (func $main))
        (export "memory" (memory 0))
    )
"#;

fn deploy_exec_balance_contract(ctx: &mut EvmTestingContext) -> Address {
    let wasm = parse_str(EXEC_BALANCE_DOS_WAT).expect("invalid wat");
    let deployer = Address::ZERO;
    let deploy_result = TxBuilder::create(ctx, deployer, wasm.into())
        .gas_price(0)
        .gas_limit(50_000_000)
        .exec();
    assert!(
        deploy_result.is_success(),
        "failed to deploy exec test contract: {deploy_result:?}"
    );
    calc_create_address(&deployer, 0)
}

fn call_with_len(
    ctx: &mut EvmTestingContext,
    contract: Address,
    len: u32,
) -> revm::context::result::ExecutionResult<fluentbase_revm::RwasmHaltReason> {
    let calldata = Bytes::from(len.to_le_bytes().to_vec());
    TxBuilder::call(ctx, Address::ZERO, contract, None)
        .gas_price(0)
        .gas_limit(22_000)
        .input(calldata)
        .exec()
}

#[test]
fn exec_balance_accepts_unbounded_inputs_without_gas_cost() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let contract = deploy_exec_balance_contract(&mut ctx);

    // Ensure gas cost is independent of input length.
    const SMALL_LEN: u32 = 32;
    // 32MB of unmetered input copying
    const LARGE_LEN: u32 = 32 * 1024 * 1024;

    let small = call_with_len(&mut ctx, contract, SMALL_LEN);
    assert!(
        small.is_success(),
        "baseline call unexpectedly failed: {small:?}"
    );

    let large = call_with_len(&mut ctx, contract, LARGE_LEN);
    assert!(large.is_halt(), "large call should halt: {large:?}");
}
