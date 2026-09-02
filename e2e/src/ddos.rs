use crate::EvmTestingContextWithGenesis;
use fluentbase_revm::RwasmHaltReason;
use fluentbase_sdk::{
    calc_create_address, Address, Bytes, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST, FUEL_DENOM_RATE,
    OUTPUT_WORD_FUEL_SURCHARGE,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use revm::context::result::ExecutionResult;
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

const REPEATED_WRITE_OUTPUT_WAT: &str = r#"
    (module
        (import "fluentbase_v1preview" "_read" (func $_read (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_write" (func $_write (param i32 i32)))
        (memory (export "memory") 16) ;; 1 MiB
        (func (export "deploy"))
        (func (export "main") (local $remaining i32)
            ;; Read the write count from the first four calldata bytes. Runtime context occupies
            ;; the first 1024 input bytes exposed through `_read`.
            i32.const 0
            i32.const 1024
            i32.const 4
            call $_read
            i32.const 0
            i32.load
            local.set $remaining

            loop $write
                ;; Reuse the same valid 1 MiB guest-memory range for every append.
                i32.const 0
                i32.const 1048576
                call $_write

                local.get $remaining
                i32.const 1
                i32.sub
                local.tee $remaining
                br_if $write
            end
        )
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
    TxBuilder::call(ctx, contract)
        .caller(Address::ZERO)
        .gas_price(0)
        .gas_limit(50_000)
        .input(calldata)
        .exec()
}

#[test]
fn ddos_balance_rejects_huge_input_without_memory_copy() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let contract = deploy_exec_balance_contract(&mut ctx);

    // Ensure gas cost is independent of input length.
    const SMALL_LEN: u32 = 20;
    // 32MB of unmetered input copying
    const LARGE_LEN: u32 = 20 * 1024 * 1024;

    let small = call_with_len(&mut ctx, contract, SMALL_LEN);
    assert!(
        small.is_success(),
        "baseline call unexpectedly failed: {small:?}"
    );

    let large = call_with_len(&mut ctx, contract, LARGE_LEN);
    assert!(large.is_halt(), "large call should halt: {large:?}");
}

#[test]
fn ddos_repeated_write_exhausts_fuel_before_aggregate_output_growth() {
    const CHUNK_BYTES: usize = 1024 * 1024;
    const AFFORDABLE_WRITE_COUNT: u32 = 29;
    const EXHAUSTING_WRITE_COUNT: u32 = 30;
    const MAX_BLOCK_GAS: u64 = 100_000_000;

    let wasm = parse_str(REPEATED_WRITE_OUTPUT_WAT).expect("invalid repeated-write wat");
    let deployer = Address::ZERO;
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deploy_result = TxBuilder::create(&mut ctx, deployer, wasm.into())
        .gas_price(0)
        .gas_limit(1_000_000)
        .exec();
    assert!(
        deploy_result.is_success(),
        "failed to deploy repeated-write contract: {deploy_result:?}"
    );

    let contract = calc_create_address(&deployer, 0);
    let affordable = TxBuilder::call(&mut ctx, contract)
        .caller(deployer)
        .gas_price(0)
        .gas_limit(MAX_BLOCK_GAS)
        .input(Bytes::copy_from_slice(
            &AFFORDABLE_WRITE_COUNT.to_le_bytes(),
        ))
        .exec();
    assert!(
        affordable.is_success(),
        "fuel should permit {AFFORDABLE_WRITE_COUNT} MiB of output: {affordable:?}"
    );

    let output_len = affordable
        .output()
        .expect("successful call must return output")
        .len();
    assert_eq!(output_len, CHUNK_BYTES * AFFORDABLE_WRITE_COUNT as usize);

    // Every append pays both the generic copy cost and the output-retention surcharge. The next
    // 1 MiB write exceeds the block fuel budget even though it reuses the same guest-memory range.
    let words_per_write = (CHUNK_BYTES as u64).div_ceil(32);
    let fuel_per_write = COPY_BASE_FUEL_COST as u64
        + (COPY_WORD_FUEL_COST as u64 + OUTPUT_WORD_FUEL_SURCHARGE as u64) * words_per_write;
    let gas_per_write = fuel_per_write.div_ceil(FUEL_DENOM_RATE);
    assert!(
        gas_per_write * AFFORDABLE_WRITE_COUNT as u64 <= MAX_BLOCK_GAS,
        "the affordable case must fit within the block gas budget"
    );
    assert!(
        gas_per_write * EXHAUSTING_WRITE_COUNT as u64 > MAX_BLOCK_GAS,
        "the exhausting case must exceed the block gas budget"
    );

    let exhausting = TxBuilder::call(&mut ctx, contract)
        .caller(deployer)
        .gas_price(0)
        .gas_limit(MAX_BLOCK_GAS)
        .input(Bytes::copy_from_slice(
            &EXHAUSTING_WRITE_COUNT.to_le_bytes(),
        ))
        .exec();
    assert!(
        matches!(
            exhausting,
            ExecutionResult::Halt {
                reason: RwasmHaltReason::OutOfFuel,
                ..
            }
        ),
        "the output surcharge must halt aggregate growth: {exhausting:?}"
    );
}
