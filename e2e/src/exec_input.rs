use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{
    calc_create_address, syscall::SYSCALL_ID_CALL, Address, Bytes, STATE_MAIN, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use std::{
    fmt::Write,
    time::{Duration, Instant},
};

fn encode_bytes_for_wat(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for byte in bytes {
        let _ = write!(&mut out, "\\{:02x}", byte);
    }
    out
}

fn callee_contract_wat() -> String {
    r#"
        (module
        (import "fluentbase_v1preview" "_exit"                (func $_exit                (param i32)))
        (import "fluentbase_v1preview" "_input_size"          (func $_input_size          (result i32)))
        (import "fluentbase_v1preview" "_write"               (func $_write               (param i32 i32)))
        (func $main
            i32.const 0          ;; ptr
            call $_input_size
            i32.store

            i32.const 0          ;; ptr
            i32.const 4          ;; len
            call $_write

            ;; exit
            i32.const 0
            call $_exit
        )
        (memory 1)   ;; 1 page = 64 KiB
        (export "main"   (func $main))
        (export "memory" (memory 0))
    )
    "#.to_owned()
}

fn caller_contract_wat(callee: Address, payload_size: usize) -> String {
    // Build data section
    let mut data_section = Vec::new();
    data_section.extend_from_slice(&SYSCALL_ID_CALL.0); // [0..32]
    data_section.extend_from_slice(callee.as_slice()); // [32..52]
    data_section.extend_from_slice(&U256::ZERO.as_le_slice()); // [52..84] value=0
    data_section.extend(vec![0xAB; payload_size]); // [84..84+N] payload

    let data_section_wat = encode_bytes_for_wat(&data_section);

    // EXEC parameters
    let input_ptr = 32; // Start from address
    let input_len = 52 + payload_size; // address(20) + value(32) + payload

    // Calculate memory pages
    let page_size = 65536;
    let total_bytes = data_section.len();
    let required_pages = (total_bytes + page_size - 1) / page_size;
    let memory_pages = required_pages.max(1);

    format!(
        r#"
        (module
            (import "fluentbase_v1preview" "_exec"        (func $_exec        (param i32 i32 i32 i32 i32) (result i32)))
            (import "fluentbase_v1preview" "_exit"        (func $_exit        (param i32)))
            (import "fluentbase_v1preview" "_output_size" (func $_output_size (result i32)))
            (import "fluentbase_v1preview" "_read_output" (func $_read_output (param i32 i32 i32)))
            (import "fluentbase_v1preview" "_write"       (func $_write       (param i32 i32)))

            (memory {memory_pages})
            (data (i32.const 0) "{data_section_wat}")

            (func $main (local $output_ptr i32) (local $exec_result i32)
                ;; Set output pointer
                i32.const {output_offset}
                local.set $output_ptr

                ;; Call _exec with SYSCALL_ID_CALL
                ;; THIS SHOULD TRIGGER QUADRATIC FUEL CHARGING!
                (local.set $exec_result
                    (call $_exec
                        (i32.const 0)              ;; code_hash_ptr (SYSCALL_ID_CALL at [0..32])
                        (i32.const {input_ptr})    ;; input_ptr (address at [32..])
                        (i32.const {input_len})    ;; input_len = 52 + payload ← KEY PARAMETER!
                        (i32.const 0)              ;; fuel_ptr (0 = unlimited)
                        (i32.const {STATE_MAIN})   ;; state
                    )
                )

                ;; Check if exec succeeded (result should be 0 for success)
                ;; If failed, just exit
                (if (i32.ne (local.get $exec_result) (i32.const 0))
                    (then
                        (call $_exit (i32.const 1))
                    )
                )

                ;; Read output from callee
                (call $_read_output
                    (local.get $output_ptr)    ;; destination
                    (i32.const 0)              ;; source offset
                    (call $_output_size)       ;; size
                )

                ;; Forward output
                (call $_write
                    (local.get $output_ptr)    ;; source
                    (i32.const 4)              ;; size (4 bytes)
                )

                ;; Exit
                (call $_exit (i32.const 0))
            )

            (export "main" (func $main))
            (export "memory" (memory 0))
        )
        "#,
        memory_pages = memory_pages,
        data_section_wat = data_section_wat,
        input_ptr = input_ptr,
        input_len = input_len,
        output_offset = ((total_bytes + 63) / 64) * 64,
        STATE_MAIN = STATE_MAIN,
    )
}

fn deploy_contract(
    ctx: &mut EvmTestingContext,
    deployer: Address,
    nonce: u64,
    wat: &str,
) -> Address {
    let wasm = wat::parse_str(&wat).expect("Failed to parse WAT");
    let mut builder = TxBuilder::create(ctx, deployer, wasm.into());
    let result = builder.exec();
    assert!(result.is_success(), "failed to deploy contract: {result:?}");
    calc_create_address(&deployer, nonce)
}

fn measure_call_gas(payload_size: usize) -> (u64, Duration) {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deployer = Address::ZERO;

    // Deploy callee
    let callee_wat = callee_contract_wat();
    let callee = deploy_contract(&mut ctx, deployer, 0, &callee_wat);

    // Deploy caller
    let caller_wat = caller_contract_wat(callee, payload_size);
    let caller = deploy_contract(&mut ctx, deployer, 1, &caller_wat);

    {
        // Call callee contract directly
        let call = ctx.call_evm_tx(
            deployer,
            callee,
            Bytes::from(vec![1u8; payload_size]),
            Some(500_000_000),
            None,
        );
        let output = call.output().unwrap();

        let input_size = u32::from_le_bytes(output[0..4].try_into().unwrap());
        let expected_input = payload_size as u32 + 1024; // payload + SharedContext length
        assert_eq!(
            input_size, expected_input,
            "direct call to callee returns unexpected result: {call:?}"
        );
    }

    // Execute and measure
    let start = Instant::now();
    let call = TxBuilder::call(&mut ctx, deployer, caller, None)
        .gas_limit(500_000_000)
        .exec();
    let elapsed = start.elapsed();

    assert!(call.is_success(), "call failed: {:?}", call);

    // Verify output
    let output = call.output().cloned().unwrap_or_default();

    assert_eq!(output.len(), 4, "expected 4 bytes output");
    let input_size = u32::from_le_bytes(output[0..4].try_into().unwrap());
    let expected_input = payload_size + 1024; // payload + SharedContext
    assert_eq!(input_size, expected_input as u32, "input size mismatch");

    let gas_used = call.gas_used();

    (gas_used, elapsed)
}

fn calc_evm_memory_expansion_cost(payload_size: usize) -> u64 {
    let words = payload_size / 32;
    (3 * words + words * words / 512) as u64
}

#[test]
fn test_exec_quadratic_charging() {
    let payload_sizes = [
        0,            // 0 bytes - baseline
        1usize << 10, // 1 KiB
        1usize << 11, // 2 KiB
        1usize << 12, // 4 KiB
        1usize << 13, // 8 KiB
        1usize << 16, // 64 KiB
        1usize << 20, // 1 MiB
    ];

    let mut observations = Vec::with_capacity(payload_sizes.len());

    for &payload in &payload_sizes {
        let (total_gas, elapsed) = measure_call_gas(payload);
        observations.push((payload, total_gas, elapsed));
    }

    // Baseline: gas for 0 payload (overhead)
    let baseline = observations[0].1;
    const TOLERANCE: f64 = 0.01; // 1% tolerance for rounding

    for &(payload, total_gas, _) in &observations[1..] {
        let input_gas = total_gas - baseline;
        let words = (payload + 31) / 32;
        let expected = (3 * words + words * words / 512) as u64;
        let diff = input_gas.abs_diff(expected);

        assert!(
            diff <= (expected as f64 * TOLERANCE) as u64,
            "payload={}: gas={}, expected={}, diff={}",
            payload,
            input_gas,
            expected,
            diff
        );
    }
}
