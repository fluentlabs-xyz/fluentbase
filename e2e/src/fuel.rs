//! Fuel consumption tests for system precompiles without embedded fuel metering (ENGINE_METERED_PRECOMPILES).
//!
//! Measures fuel consumed by precompiles to calibrate gas pricing.
//!
//! ## Usage
//! ```bash
//! cargo test fuel_ --release -- --nocapture
//! cargo test fuel_ --release --features wasmtime -- --nocapture
//! ```

use crate::EvmTestingContextWithGenesis;
use fluentbase_runtime::{default_runtime_executor, RuntimeContext, RuntimeExecutor};
use fluentbase_sdk::{
    bytes::BytesMut, compile_wasm_to_rwasm_with_config, default_compilation_config,
    import_linker_v1_preview, BytecodeOrHash, ContractContextV1, SharedContextInputV1,
    PRECOMPILE_NITRO_VERIFIER, PRECOMPILE_WEBAUTHN_VERIFIER, STATE_MAIN,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use revm::primitives::{keccak256, Address, Bytes, U256};
use std::time::Instant;

#[test]
fn fuel_nitro_verifier_evm_ctx() {
    let input = hex::decode(include_bytes!(
        "../../contracts/nitro/attestation-example.hex"
    ))
    .expect("invalid hex");

    let caller = Address::ZERO;
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(caller, U256::from(1e18 as u128));

    // Warmup for fair comparison
    ctx.warmup_bytecode(PRECOMPILE_NITRO_VERIFIER);

    // Execute via EVM
    let start = Instant::now();
    let result = TxBuilder::call(&mut ctx, caller, PRECOMPILE_NITRO_VERIFIER, None)
        .input(Bytes::from(input))
        .gas_limit(100_000_000_000)
        .timestamp(1695050165) // ensure correct block timestamp to match certificate time window
        .exec();
    let elapsed = start.elapsed();

    // Report
    #[cfg(feature = "wasmtime")]
    let runtime = "wasmtime";
    #[cfg(not(feature = "wasmtime"))]
    let runtime = "rwasm";

    println!("=== nitro_verifier ({}) ===", runtime);
    println!("  gas_used:  {:>12}", result.gas_used());
    println!("  time:      {:>12.2?}", elapsed);
    println!("  success:   {:>12}", result.is_success());

    if !result.is_success() {
        println!("  output: {:?}", result.output());
    }

    assert!(result.is_success(), "execution failed: {:?}", result);
    assert!(result.gas_used() > 0, "no gas consumed");
}
