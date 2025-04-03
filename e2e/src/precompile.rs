extern crate test;

use crate::utils::{EvmTestingContext, TxBuilder};
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{address, PRECOMPILE_NITRO_VERIFIER};
use std::time::Instant;

#[test]
fn test_nitro_precompile() {
    let caller = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let bytecode = include_bytes!("../../contracts/nitro/lib.wasm");
    let mut ctx = EvmTestingContext::default();
    let address = ctx.deploy_evm_tx(caller, bytecode.into());
    let address = PRECOMPILE_NITRO_VERIFIER;

    let start = Instant::now();
    let mut total_gas = 0;

    let attestation_doc: Vec<u8> = hex::decode(include_bytes!(
        "../../contracts/nitro/attestation-example.hex"
    ))
    .unwrap()
    .into();
    let result = TxBuilder::call(&mut ctx, caller, address, None)
        .enable_rwasm_proxy()
        .input(attestation_doc.into())
        .gas_limit(1_000_000_000)
        .exec();
    if !result.is_success() {
        panic!("attestation verification failed, result: {:?}", result);
    }
    total_gas += result.gas_used();
    println!(
        "NitroVerifier.wasm: time={:.2?}, gas={}",
        start.elapsed(),
        total_gas
    );
    panic!("FINISHED");
}
