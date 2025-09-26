use crate::EvmTestingContextWithGenesis;
use fluentbase_revm::RwasmHaltReason;
use fluentbase_sdk::{Address, Bytes};
use fluentbase_testing::{EvmTestingContext, HostTestingContextNativeAPI, TxBuilder};
use fluentbase_types::{
    calc_create_address, CHARGE_FUEL_BASE_COST, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST,
    KECCAK_BASE_FUEL_COST, KECCAK_WORD_FUEL_COST, LOW_FUEL_COST, SECP256K1_RECOVER_BASE_FUEL_COST,
};
use revm::{
    context::result::ExecutionResult, interpreter::gas::calculate_initial_tx_gas,
    primitives::hardfork::SpecId,
};

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

fn run_main(main_function_wat: &str, call_data_size: usize) -> ExecutionResult<RwasmHaltReason> {
    let wat = WAT_TEMPLATE.replace("            {{MAIN_BODY}}", main_function_wat);
    let wasm = wat::parse_str(&wat).unwrap();
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deployer: Address = Address::ZERO;
    let mut builder = TxBuilder::create(&mut ctx, deployer, wasm.into());
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
    result
}

/// Calculates how much gas is consumed by the builtins
fn run_twice_and_find_gas_difference(wat: &str, call_data_size: usize) -> u64 {
    let result1 = run_main(wat, call_data_size);
    assert!(result1.is_success());
    let init_gas =
        calculate_initial_tx_gas(SpecId::PRAGUE, &vec![0u8; call_data_size], false, 0, 0, 0);
    result1.gas_used() - init_gas.initial_gas
}

#[test]
fn test_keccak_builtin() {
    let main = r#"
        i32.const 0        ;; data offset
        i32.const 123000   ;; data length
        i32.const 0        ;; output offset
        call $_keccak256
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = KECCAK_BASE_FUEL_COST + KECCAK_WORD_FUEL_COST * ((123000 + 31) / 32);
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_write_builtin() {
    let main = r#"
        i32.const 0
        i32.const 123000
        call $_write
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * ((123000 + 31) / 32);
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_write_builtin_overflow() {
    let main_without_overflow = r#"
        i32.const 0
        i32.const 100
        call $_write
    "#;
    let main_with_overflow = r#" 
        i32.const 0
        i32.const 300000 
        call $_write
    "#; // excessively large data size argument
    let result1 = run_main(main_without_overflow, 0);
    let result2 = run_main(main_with_overflow, 0);

    assert!(matches!(result1, ExecutionResult::Success { .. }));
    assert!(matches!(
        result2,
        ExecutionResult::Halt {
            reason: RwasmHaltReason::IntegerOverflow,
            ..
        }
    ));
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
    let gas = run_twice_and_find_gas_difference(main, 1_000) - 30_000;
    let expected_fuel =
        COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * ((800 + 31) / 32) + CHARGE_FUEL_BASE_COST;
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_debug_log_builtin() {
    let main = r#"
        i32.const 0        ;; data_ptr
        i32.const 123000   ;; len 123KiB
        call $_debug_log
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * ((123000 + 31) / 32);
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_output_size_builtin() {
    let main = r#"
        call $_output_size
        drop
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = LOW_FUEL_COST;
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_state_builtin() {
    let main = r#"
        call $_state
        drop
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = LOW_FUEL_COST;
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_fuel_builtin() {
    let main = r#"
        call $_fuel
        drop
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = LOW_FUEL_COST;
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_charge_fuel_builtin() {
    let main = r#"
        i64.const 0        ;; amount
        call $_charge_fuel
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = LOW_FUEL_COST;
    assert_eq!(gas, expected_fuel as u64 / 1000);
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
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = SECP256K1_RECOVER_BASE_FUEL_COST;
    assert_eq!(gas, expected_fuel as u64 / 1000);
}

#[test]
fn test_exit_builtin() {
    let main = r#"
        i32.const 0        ;; output offset
        call $_exit
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    assert_eq!(gas, 0); // no gas consumed for exit binding
}
