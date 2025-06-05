use fluentbase_sdk_testing::{EvmTestingContext, TxBuilder};
use fluentbase_sdk::{Address, Bytes};
use fluentbase_sdk_testing::HostTestingContextNativeAPI;
use fluentbase_types::{
    calc_create_address,
    CHARGE_FUEL_BASE_COST,
    COPY_BASE_FUEL_COST,
    COPY_WORD_FUEL_COST,
    KECCAK_BASE_FUEL_COST,
    KECCAK_WORD_FUEL_COST,
    MINIMAL_BASE_FUEL_COST,
    SECP256K1_RECOVER_BASE_FUEL_COST,
};
use revm::context::result::ExecutionResult;

const WAT_TEMPLATE: &str = r#"
    (module
        (import "fluentbase_v1preview" "_charge_fuel"         (func $_charge_fuel         (param i64)))
        (import "fluentbase_v1preview" "_charge_fuel_manually"(func $_charge_fuel_manually(param i64 i64) (result i64)))
        (import "fluentbase_v1preview" "_debug_log"           (func $_debug_log           (param i32 i32)))
        (import "fluentbase_v1preview" "_exec"                (func $_exec                (param i32 i32 i32 i32 i32) (result i32)))
        (import "fluentbase_v1preview" "_exit"                (func $_exit                (param i32)))
        (import "fluentbase_v1preview" "_forward_output"      (func $_forward_output      (param i32 i32)))
        (import "fluentbase_v1preview" "_fuel"                (func $_fuel                (result i64)))
        (import "fluentbase_v1preview" "_input_size"          (func $_input_size          (result i32)))
        (import "fluentbase_v1preview" "_keccak256"           (func $_keccak256           (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_output_size"         (func $_output_size         (result i32)))
        (import "fluentbase_v1preview" "_preimage_copy"       (func $_preimage_copy       (param i32 i32)))
        (import "fluentbase_v1preview" "_preimage_size"       (func $_preimage_size       (param i32) (result i32)))
        (import "fluentbase_v1preview" "_read"                (func $_read                (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_read_output"         (func $_read_output         (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_resume"              (func $_resume              (param i32 i32 i32 i32 i32) (result i32)))
        (import "fluentbase_v1preview" "_secp256k1_recover"   (func $_secp256k1_recover   (param i32 i32 i32 i32) (result i32)))
        (import "fluentbase_v1preview" "_state"               (func $_state               (result i32)))
        (import "fluentbase_v1preview" "_write"               (func $_write               (param i32 i32)))
        (func $main
            {{MAIN_BODY}}
        )
        (memory 10)   ;; 1 page = 64 KiB
        (export "main"   (func $main))
        (export "memory" (memory 0))
    )
"#;

fn run_main(
    main_function_wat: &str,
    call_data_size: usize,
    builtins_consume_fuel: bool,
) -> ExecutionResult {
    let wat = WAT_TEMPLATE.replace("            {{MAIN_BODY}}", main_function_wat);
    let wasm = wat::parse_str(&wat).unwrap();
    let mut ctx = EvmTestingContext::default();
    let deployer: Address = Address::ZERO;
    let mut builder = TxBuilder::create(&mut ctx, deployer, wasm.into());
    if builtins_consume_fuel {
        builder = builder.disable_builtins_consume_fuel();
    }
    let result = builder.exec();
    assert!(result.is_success(), "failed to deploy contract");
    let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&deployer, 0);
    let result = ctx.call_evm_tx(
        deployer,
        contract_address,
        Bytes::from(vec![0u8; call_data_size]),
        None,
        None,
    );
    assert!(result.is_success());
    result
}

/// Calculates how much gas is consumed by the builtins
fn measure_gas_used_by_builtins(wat: &str, call_data_size: usize) -> u64 {
    println!("Code: {}", wat);
    println!("-------------");
    println!("Running with builtins_consume_fuel=true");
    let result1 = run_main(wat, call_data_size, true);
    println!("RESULT: {:?}", result1);
    println!("-------------");
    println!("Running with builtins_consume_fuel=false");
    let result2 = run_main(wat, call_data_size, false);
    println!("RESULT: {:?}", result2);
    println!("-------------");
    assert!(result1.gas_used() >= result2.gas_used());
    result1.gas_used() - result2.gas_used()
}

#[test]
fn test_keccak_builtin() {
    let main = r#"
        i32.const 0        ;; data offset
        i32.const 307200   ;; data length 300KiB
        i32.const 0        ;; output offset
        call $_keccak256
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = KECCAK_BASE_FUEL_COST + KECCAK_WORD_FUEL_COST * ((307200 + 31) / 32);
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_write_builtin() {
    let main = r#"
        i32.const 0
        i32.const 307200
        call $_write
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * ((307200 + 31) / 32);
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_read_builtin() {
    let main = r#"
        i64.const 30_000_000
        call $_charge_fuel   ;; pre-spend gas to neutralize gas_floor adjustment that depends on calldata size.
        i32.const 0
        i32.const 0
        i32.const 800
        call $_read
    "#;
    let gas = measure_gas_used_by_builtins(main, 1_000);
    let expected_fuel =
        COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * ((800 + 31) / 32) + CHARGE_FUEL_BASE_COST;
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_debug_log_builtin() {
    let main = r#"
        i32.const 0        ;; data_ptr
        i32.const 307200   ;; len 300 KiB
        call $_debug_log
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * ((307200 + 31) / 32);
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_output_size_builtin() {
    let main = r#"
        call $_output_size
        drop
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = MINIMAL_BASE_FUEL_COST;
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_state_builtin() {
    let main = r#"
        call $_state
        drop
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = MINIMAL_BASE_FUEL_COST;
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_fuel_builtin() {
    let main = r#"
        call $_fuel
        drop
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = MINIMAL_BASE_FUEL_COST;
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_charge_fuel_builtin() {
    let main = r#"
        i64.const 0        ;; amount
        call $_charge_fuel
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = MINIMAL_BASE_FUEL_COST;
    assert_eq!(gas, expected_fuel / 1000);
}

#[test]
fn test_secp256k1_recover_builtin() {
    let main = r#"
        i32.const 0        ;; digest_ptr
        i32.const 0        ;; sig_ptr
        i32.const 0        ;; out_ptr
        i32.const 0        ;; rec_id
        call $_secp256k1_recover
        drop
    "#;
    let gas = measure_gas_used_by_builtins(main, 0);
    let expected_fuel = SECP256K1_RECOVER_BASE_FUEL_COST;
    assert_eq!(gas, expected_fuel / 1000);
}
