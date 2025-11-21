use crate::EvmTestingContextWithGenesis;
use fluentbase_revm::RwasmHaltReason;
use fluentbase_sdk::{
    calc_create_address, Address, Bytes, CHARGE_FUEL_BASE_COST, COPY_BASE_FUEL_COST,
    COPY_WORD_FUEL_COST, DEBUG_LOG_BASE_FUEL_COST, DEBUG_LOG_WORD_FUEL_COST, FUEL_DENOM_RATE,
    KECCAK_BASE_FUEL_COST, KECCAK_WORD_FUEL_COST, LOW_FUEL_COST,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use revm::{
    context::result::ExecutionResult, interpreter::gas::calculate_initial_tx_gas,
    primitives::hardfork::SpecId,
};
use rwasm::CALL_FUEL_COST;

const WAT_TEMPLATE: &str = r#"
    (module
        (import "fluentbase_v1preview" "_charge_fuel"         (func $_charge_fuel         (param i64)))
        (import "fluentbase_v1preview" "_debug_log"           (func $_debug_log           (param i32 i32)))
        (import "fluentbase_v1preview" "_exec"                (func $_exec                (param i32 i32 i32 i32 i32) (result i32)))
        (import "fluentbase_v1preview" "_exit"                (func $_exit                (param i32)))
        (import "fluentbase_v1preview" "_forward_output"      (func $_forward_output      (param i32 i32)))
        (import "fluentbase_v1preview" "_fuel"                (func $_fuel                (result i64)))
        (import "fluentbase_v1preview" "_input_size"          (func $_input_size          (result i32)))
        (import "fluentbase_v1preview" "_keccak256"           (func $_keccak256           (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_output_size"         (func $_output_size         (result i32)))
        (import "fluentbase_v1preview" "_read"                (func $_read                (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_read_output"         (func $_read_output         (param i32 i32 i32)))
        (import "fluentbase_v1preview" "_resume"              (func $_resume              (param i32 i32 i32 i32 i32) (result i32)))
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
    println!("{:?}", result);
    assert!(result.is_success(), "failed to deploy contract");
    let contract_address = calc_create_address(&deployer, 0);
    let result = ctx.call_evm_tx(
        deployer,
        contract_address,
        Bytes::from(vec![0u8; call_data_size]),
        None,
        None,
    );
    result
}

/// Converts fuel to gas, accounting for testnet vs mainnet behavior
fn fuel_to_gas(fuel: u32) -> u64 {
    #[cfg(feature = "fluent-testnet")]
    {
        // Testnet: floor division (allows free operations below threshold)
        fuel as u64 / FUEL_DENOM_RATE
    }
    #[cfg(not(feature = "fluent-testnet"))]
    {
        // Mainnet: ceiling division (no free operations)
        (fuel as u64 + FUEL_DENOM_RATE - 1) / FUEL_DENOM_RATE
    }
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
    let words = (123000 + 31) / 32;
    let expected_fuel = KECCAK_BASE_FUEL_COST + KECCAK_WORD_FUEL_COST * words + CALL_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_write_builtin() {
    let main = r#"
        i32.const 0
        i32.const 123000
        call $_write
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let words = (123000 + 31) / 32;
    let expected_fuel = COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * words + CALL_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
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
    // 30_000_000 fuel / FUEL_DENOM_RATE = 1_500_000 gas
    let gas_offset = fuel_to_gas(30_000_000);
    let gas = run_twice_and_find_gas_difference(main, 1_000).saturating_sub(gas_offset);
    let words = (800 + 31) / 32;
    let expected_fuel =
        COPY_BASE_FUEL_COST + COPY_WORD_FUEL_COST * words + CHARGE_FUEL_BASE_COST + CALL_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_debug_log_builtin() {
    let main = r#"
        i32.const 0        ;; data_ptr
        i32.const 123000   ;; len 123KiB
        call $_debug_log
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let words = (123000 + 31) / 32;
    let expected_fuel =
        DEBUG_LOG_BASE_FUEL_COST + DEBUG_LOG_WORD_FUEL_COST * words + CALL_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_output_size_builtin() {
    let main = r#"
        call $_output_size
        drop
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);

    // OUTPUT_SIZE syscall uses LOW_FUEL_COST
    let expected_fuel = CALL_FUEL_COST + LOW_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_state_builtin() {
    let main = r#"
        call $_state
        drop
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    // STATE syscall uses LOW_FUEL_COST
    let expected_fuel = CALL_FUEL_COST + LOW_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_fuel_builtin() {
    let main = r#"
        call $_fuel
        drop
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    // FUEL syscall uses LOW_FUEL_COST
    let expected_fuel = CALL_FUEL_COST + LOW_FUEL_COST;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_charge_fuel_builtin() {
    // Every WASM 'call' instruction costs CALL_FUEL_COST (1 fuel), added by the rwasm translator.
    // So _charge_fuel total cost = CALL_FUEL_COST (for the 'call' instruction itself)
    //                             + CHARGE_FUEL_BASE_COST (builtin's base cost)
    //                             + argument (amount of fuel to charge)

    // Multiple calls with zero argument - shows base costs accumulate per call
    let main = r#"
        i64.const 0
        call $_charge_fuel
        i64.const 0
        call $_charge_fuel
        i64.const 0
        call $_charge_fuel
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = 3 * (CALL_FUEL_COST + CHARGE_FUEL_BASE_COST);
    assert_eq!(gas, fuel_to_gas(expected_fuel));

    // Call with argument - shows that argument adds to the base costs
    let main = r#"
        i64.const 500
        call $_charge_fuel
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    let expected_fuel = CALL_FUEL_COST + CHARGE_FUEL_BASE_COST + 500;
    assert_eq!(gas, fuel_to_gas(expected_fuel));
}

#[test]
fn test_exit_builtin() {
    let main = r#"
        i32.const 0        ;; output offset
        call $_exit
    "#;
    let gas = run_twice_and_find_gas_difference(main, 0);
    // Exit doesn't consume fuel, only the call instruction
    assert_eq!(gas, fuel_to_gas(CALL_FUEL_COST));
}